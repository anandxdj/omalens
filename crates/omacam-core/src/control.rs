//! Authenticated reconnect protocol state.
//!
//! The transport must be TLS 1.3 and the phone must pin the exact desktop
//! certificate saved during pairing. This module proves possession of the
//! already-trusted phone identity key using a fresh, single-use challenge. A
//! successful reconnect authenticates control only; it never authorizes camera
//! capture.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const CONTROL_PROTOCOL_VERSION: u8 = 1;
pub const CONTROL_CHALLENGE_LIFETIME_MS: u64 = 10_000;
pub const CONTROL_FEATURES: u32 = 0;

const SESSION_ID_BYTES: usize = 16;
const CHALLENGE_BYTES: usize = 32;
const CERTIFICATE_DIGEST_BYTES: usize = 32;
const PHONE_NONCE_BYTES: usize = 32;
const MAX_PUBLIC_KEY_DER_BYTES: usize = 256;
const MAX_SIGNATURE_DER_BYTES: usize = 96;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlHello {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(rename = "min_v")]
    minimum_version: u8,
    #[serde(rename = "max_v")]
    maximum_version: u8,
    #[serde(rename = "features")]
    features: u32,
    #[serde(rename = "required")]
    required_features: u32,
    #[serde(rename = "sid")]
    session_id: String,
    challenge: String,
    #[serde(rename = "cert")]
    certificate_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlProof {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(rename = "v")]
    pub version: u8,
    #[serde(rename = "features")]
    pub features: u32,
    #[serde(rename = "sid")]
    pub session_id: String,
    #[serde(rename = "key")]
    pub phone_public_key_sha256: String,
    #[serde(rename = "nonce")]
    pub phone_nonce: String,
    #[serde(rename = "sig")]
    pub signature_der: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedControl {
    pub session_id: String,
    pub phone_key_sha256: [u8; 32],
    pub version: u8,
    pub features: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    AwaitingProof,
    Authenticated,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlError {
    RandomSourceUnavailable,
    UnsupportedVersion,
    UnsupportedRequiredFeature,
    InvalidMessageType,
    InvalidFieldLength,
    WrongSession,
    WrongIdentity,
    InvalidPublicKey,
    InvalidSignature,
    Expired,
    ReplayRejected,
    Rejected,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::RandomSourceUnavailable => "secure random source unavailable",
            Self::UnsupportedVersion => "unsupported control protocol version",
            Self::UnsupportedRequiredFeature => "required control feature is unsupported",
            Self::InvalidMessageType => "invalid control message type",
            Self::InvalidFieldLength => "control field has an invalid length",
            Self::WrongSession => "control session does not match",
            Self::WrongIdentity => "phone identity does not match trusted peer",
            Self::InvalidPublicKey => "trusted phone public key is invalid",
            Self::InvalidSignature => "phone reconnect proof is invalid",
            Self::Expired => "control challenge expired",
            Self::ReplayRejected => "control proof was already used",
            Self::Rejected => "control session was rejected",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ControlError {}

pub struct ControlSession {
    hello: ControlHello,
    created_monotonic_ms: u64,
    state: ControlState,
}

impl ControlSession {
    /// Creates a fresh, bounded reconnect challenge tied to the TLS certificate.
    ///
    /// # Errors
    ///
    /// Returns an error when the certificate is empty or the operating-system
    /// secure random source is unavailable.
    pub fn create(certificate_der: &[u8], now_monotonic_ms: u64) -> Result<Self, ControlError> {
        if certificate_der.is_empty() {
            return Err(ControlError::InvalidFieldLength);
        }
        let mut session_id = [0_u8; SESSION_ID_BYTES];
        let mut challenge = [0_u8; CHALLENGE_BYTES];
        getrandom::fill(&mut session_id).map_err(|_| ControlError::RandomSourceUnavailable)?;
        getrandom::fill(&mut challenge).map_err(|_| ControlError::RandomSourceUnavailable)?;
        Ok(Self {
            hello: ControlHello {
                message_type: "control_hello".to_owned(),
                minimum_version: CONTROL_PROTOCOL_VERSION,
                maximum_version: CONTROL_PROTOCOL_VERSION,
                features: CONTROL_FEATURES,
                required_features: 0,
                session_id: encode(&session_id),
                challenge: encode(&challenge),
                certificate_sha256: encode(Sha256::digest(certificate_der).as_slice()),
            },
            created_monotonic_ms: now_monotonic_ms,
            state: ControlState::AwaitingProof,
        })
    }

    #[must_use]
    pub fn hello(&self) -> &ControlHello {
        &self.hello
    }

    #[must_use]
    pub fn state(&self) -> ControlState {
        self.state
    }

    /// Verifies a proof from the paired phone and consumes this challenge.
    ///
    /// # Errors
    ///
    /// Fails closed for expired or reused challenges, incompatible protocol
    /// fields, a changed identity, malformed input, or an invalid signature.
    pub fn authenticate(
        &mut self,
        proof: &ControlProof,
        trusted_phone_public_key_der: &[u8],
        now_monotonic_ms: u64,
    ) -> Result<AuthenticatedControl, ControlError> {
        if self.state == ControlState::Authenticated {
            return Err(ControlError::ReplayRejected);
        }
        if self.state == ControlState::Rejected {
            return Err(ControlError::Rejected);
        }
        if now_monotonic_ms.saturating_sub(self.created_monotonic_ms)
            >= CONTROL_CHALLENGE_LIFETIME_MS
        {
            self.state = ControlState::Expired;
            return Err(ControlError::Expired);
        }
        if self.state == ControlState::Expired {
            return Err(ControlError::Expired);
        }

        let result = self.verify_proof(proof, trusted_phone_public_key_der);
        match result {
            Ok(authenticated) => {
                self.state = ControlState::Authenticated;
                Ok(authenticated)
            }
            Err(error) => {
                self.state = ControlState::Rejected;
                Err(error)
            }
        }
    }

    fn verify_proof(
        &self,
        proof: &ControlProof,
        trusted_phone_public_key_der: &[u8],
    ) -> Result<AuthenticatedControl, ControlError> {
        if proof.message_type != "control_proof" {
            return Err(ControlError::InvalidMessageType);
        }
        if proof.version != CONTROL_PROTOCOL_VERSION
            || proof.version < self.hello.minimum_version
            || proof.version > self.hello.maximum_version
        {
            return Err(ControlError::UnsupportedVersion);
        }
        if self.hello.required_features & !proof.features != 0 {
            return Err(ControlError::UnsupportedRequiredFeature);
        }
        if proof.session_id != self.hello.session_id {
            return Err(ControlError::WrongSession);
        }
        if trusted_phone_public_key_der.is_empty()
            || trusted_phone_public_key_der.len() > MAX_PUBLIC_KEY_DER_BYTES
        {
            return Err(ControlError::InvalidPublicKey);
        }

        let key_digest = Sha256::digest(trusted_phone_public_key_der);
        let claimed_digest =
            decode_exact(&proof.phone_public_key_sha256, CERTIFICATE_DIGEST_BYTES)?;
        if key_digest.as_slice() != claimed_digest {
            return Err(ControlError::WrongIdentity);
        }
        let phone_nonce = decode_exact(&proof.phone_nonce, PHONE_NONCE_BYTES)?;
        let signature_der = decode_bounded(&proof.signature_der, 1, MAX_SIGNATURE_DER_BYTES)?;
        let verifying_key = VerifyingKey::from_public_key_der(trusted_phone_public_key_der)
            .map_err(|_| ControlError::InvalidPublicKey)?;
        let signature =
            Signature::from_der(&signature_der).map_err(|_| ControlError::InvalidSignature)?;
        let transcript =
            self.proof_transcript(proof.version, proof.features, &claimed_digest, &phone_nonce)?;
        verifying_key
            .verify(&transcript, &signature)
            .map_err(|_| ControlError::InvalidSignature)?;

        let mut phone_key_sha256 = [0_u8; 32];
        phone_key_sha256.copy_from_slice(&key_digest);
        Ok(AuthenticatedControl {
            session_id: self.hello.session_id.clone(),
            phone_key_sha256,
            version: proof.version,
            features: proof.features & self.hello.features,
        })
    }

    fn proof_transcript(
        &self,
        version: u8,
        features: u32,
        phone_key_digest: &[u8],
        phone_nonce: &[u8],
    ) -> Result<Vec<u8>, ControlError> {
        let mut transcript = b"OMACAM-CONTROL-PROOF-V1\0".to_vec();
        append_field(&mut transcript, &[version])?;
        append_field(&mut transcript, &features.to_be_bytes())?;
        append_field(&mut transcript, self.hello.session_id.as_bytes())?;
        append_field(&mut transcript, self.hello.challenge.as_bytes())?;
        append_field(&mut transcript, self.hello.certificate_sha256.as_bytes())?;
        append_field(&mut transcript, phone_key_digest)?;
        append_field(&mut transcript, phone_nonce)?;
        Ok(transcript)
    }
}

fn append_field(target: &mut Vec<u8>, field: &[u8]) -> Result<(), ControlError> {
    let length = u32::try_from(field.len()).map_err(|_| ControlError::InvalidFieldLength)?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(field);
    Ok(())
}

fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_exact(encoded: &str, expected: usize) -> Result<Vec<u8>, ControlError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ControlError::InvalidFieldLength)?;
    if decoded.len() != expected {
        return Err(ControlError::InvalidFieldLength);
    }
    Ok(decoded)
}

fn decode_bounded(encoded: &str, minimum: usize, maximum: usize) -> Result<Vec<u8>, ControlError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ControlError::InvalidFieldLength)?;
    if !(minimum..=maximum).contains(&decoded.len()) {
        return Err(ControlError::InvalidFieldLength);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;
    use p256::pkcs8::EncodePublicKey as _;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_slice(&[seed; 32]).expect("valid test key")
    }

    fn proof(session: &ControlSession, signing_key: &SigningKey) -> ControlProof {
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let digest = Sha256::digest(public_der.as_bytes());
        let nonce = [13_u8; PHONE_NONCE_BYTES];
        let transcript = session
            .proof_transcript(CONTROL_PROTOCOL_VERSION, CONTROL_FEATURES, &digest, &nonce)
            .expect("valid transcript");
        let signature: Signature = signing_key.sign(&transcript);
        ControlProof {
            message_type: "control_proof".to_owned(),
            version: CONTROL_PROTOCOL_VERSION,
            features: CONTROL_FEATURES,
            session_id: session.hello.session_id.clone(),
            phone_public_key_sha256: encode(&digest),
            phone_nonce: encode(&nonce),
            signature_der: encode(signature.to_der().as_bytes()),
        }
    }

    #[test]
    fn trusted_key_authenticates_without_authorizing_capture() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let proof = proof(&session, &signing_key);

        let authenticated = session
            .authenticate(&proof, public_der.as_bytes(), 1_001)
            .expect("authenticate");

        assert_eq!(session.state(), ControlState::Authenticated);
        assert_eq!(authenticated.version, CONTROL_PROTOCOL_VERSION);
    }

    #[test]
    fn proof_is_single_use() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let proof = proof(&session, &signing_key);
        session
            .authenticate(&proof, public_der.as_bytes(), 1_001)
            .expect("authenticate");

        assert_eq!(
            session.authenticate(&proof, public_der.as_bytes(), 1_002),
            Err(ControlError::ReplayRejected)
        );
    }

    #[test]
    fn wrong_or_changed_phone_identity_is_rejected() {
        let trusted = key(7);
        let attacker = key(8);
        let trusted_der = trusted
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let attacker_proof = proof(&session, &attacker);

        assert_eq!(
            session.authenticate(&attacker_proof, trusted_der.as_bytes(), 1_001),
            Err(ControlError::WrongIdentity)
        );
        assert_eq!(session.state(), ControlState::Rejected);
    }

    #[test]
    fn proof_cannot_move_between_desktop_challenges() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let source = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let proof = proof(&source, &signing_key);
        let mut target = ControlSession::create(b"desktop certificate", 1_000).expect("session");

        assert_eq!(
            target.authenticate(&proof, public_der.as_bytes(), 1_001),
            Err(ControlError::WrongSession)
        );
    }

    #[test]
    fn expired_challenge_is_rejected() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let proof = proof(&session, &signing_key);

        assert_eq!(
            session.authenticate(
                &proof,
                public_der.as_bytes(),
                1_000 + CONTROL_CHALLENGE_LIFETIME_MS
            ),
            Err(ControlError::Expired)
        );
    }

    #[test]
    fn unsupported_version_fails_closed() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let mut proof = proof(&session, &signing_key);
        proof.version = CONTROL_PROTOCOL_VERSION + 1;

        assert_eq!(
            session.authenticate(&proof, public_der.as_bytes(), 1_001),
            Err(ControlError::UnsupportedVersion)
        );
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let signing_key = key(7);
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode public key");
        let mut session = ControlSession::create(b"desktop certificate", 1_000).expect("session");
        let mut proof = proof(&session, &signing_key);
        proof.signature_der = encode(&[0_u8; 8]);

        assert_eq!(
            session.authenticate(&proof, public_der.as_bytes(), 1_001),
            Err(ControlError::InvalidSignature)
        );
    }
}
