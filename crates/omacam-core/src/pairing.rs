//! Pure pairing protocol types and state transitions.
//!
//! The transport is responsible for TLS and for pinning the exact certificate
//! fingerprint carried by [`PairingInvitation`]. This module never opens a
//! socket and never persists an invitation secret.

use std::fmt;
use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

pub const PAIRING_PROTOCOL_VERSION: u8 = 1;
pub const INVITATION_LIFETIME_SECS: u64 = 120;
pub const MAX_CLOCK_SKEW_SECS: u64 = 30;
pub const MAX_INVITATION_BYTES: usize = 4_096;
pub const MAX_PEER_NAME_BYTES: usize = 64;
pub const MAX_PUBLIC_KEY_DER_BYTES: usize = 256;
pub const MAX_SIGNATURE_DER_BYTES: usize = 96;

const SESSION_ID_BYTES: usize = 16;
const INVITATION_SECRET_BYTES: usize = 32;
const CHALLENGE_BYTES: usize = 32;
const CERTIFICATE_DIGEST_BYTES: usize = 32;
const PHONE_NONCE_BYTES: usize = 32;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingInvitation {
    #[serde(rename = "v")]
    version: u8,
    #[serde(rename = "sid")]
    session_id: String,
    #[serde(rename = "token")]
    secret: String,
    #[serde(rename = "challenge")]
    challenge: String,
    #[serde(rename = "cert")]
    certificate_sha256: String,
    #[serde(rename = "endpoint")]
    endpoint: SocketAddr,
    #[serde(rename = "name")]
    desktop_name: String,
    #[serde(rename = "expires")]
    expires_at_unix_secs: u64,
}

impl fmt::Debug for PairingInvitation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingInvitation")
            .field("version", &self.version)
            .field("session_id", &self.session_id)
            .field("secret", &"[REDACTED]")
            .field("challenge", &"[REDACTED]")
            .field("certificate_sha256", &self.certificate_sha256)
            .field("endpoint", &self.endpoint)
            .field("desktop_name", &self.desktop_name)
            .field("expires_at_unix_secs", &self.expires_at_unix_secs)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingClaim {
    #[serde(rename = "v")]
    pub version: u8,
    #[serde(rename = "sid")]
    pub session_id: String,
    #[serde(rename = "token")]
    pub secret: String,
    #[serde(rename = "name")]
    pub phone_name: String,
    #[serde(rename = "key")]
    pub phone_public_key_der: String,
    #[serde(rename = "nonce")]
    pub phone_nonce: String,
    #[serde(rename = "sig")]
    pub signature_der: String,
}

impl fmt::Debug for PairingClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingClaim")
            .field("version", &self.version)
            .field("session_id", &self.session_id)
            .field("secret", &"[REDACTED]")
            .field("phone_name", &self.phone_name)
            .field("phone_public_key_der", &"[REDACTED]")
            .field("phone_nonce", &"[REDACTED]")
            .field("signature_der", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaim {
    pub phone_name: String,
    pub phone_public_key_der: Vec<u8>,
    pub phone_key_sha256: [u8; 32],
    pub sas: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvitationState {
    Available,
    AwaitingApprovals,
    Paired,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingError {
    RandomSourceUnavailable,
    MalformedInvitation,
    InvitationTooLarge,
    UnsupportedVersion,
    InvalidEndpoint,
    InvalidDesktopName,
    InvalidPeerName,
    InvalidFieldLength,
    Expired,
    AlreadyClaimed,
    Rejected,
    WrongSession,
    WrongSecret,
    InvalidPublicKey,
    InvalidSignature,
    ApprovalOutOfOrder,
}

impl fmt::Display for PairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}",
            match self {
                Self::RandomSourceUnavailable => "secure random source unavailable",
                Self::MalformedInvitation => "malformed pairing invitation",
                Self::InvitationTooLarge => "pairing invitation exceeds size limit",
                Self::UnsupportedVersion => "unsupported pairing protocol version",
                Self::InvalidEndpoint => "pairing endpoint must be a concrete unicast address",
                Self::InvalidDesktopName => "invalid desktop display name",
                Self::InvalidPeerName => "invalid phone display name",
                Self::InvalidFieldLength => "pairing field has an invalid length",
                Self::Expired => "pairing invitation expired",
                Self::AlreadyClaimed => "pairing invitation was already claimed",
                Self::Rejected => "pairing invitation was rejected",
                Self::WrongSession => "pairing session does not match",
                Self::WrongSecret => "pairing secret does not match",
                Self::InvalidPublicKey => "phone identity key is invalid",
                Self::InvalidSignature => "phone identity proof is invalid",
                Self::ApprovalOutOfOrder => "pairing approval is not currently allowed",
            }
        )
    }
}

impl std::error::Error for PairingError {}

pub struct PairingSession {
    invitation: PairingInvitation,
    created_monotonic_ms: u64,
    state: InvitationState,
    verified_claim: Option<VerifiedClaim>,
    phone_approved: bool,
    desktop_approved: bool,
}

impl PairingInvitation {
    /// Creates a bounded, short-lived invitation using the operating system CSPRNG.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsuitable endpoint/name, invalid certificate
    /// digest, time overflow, or unavailable secure randomness.
    pub fn create(
        endpoint: SocketAddr,
        desktop_name: &str,
        certificate_der: &[u8],
        now_unix_secs: u64,
    ) -> Result<Self, PairingError> {
        validate_endpoint(endpoint)?;
        validate_name(desktop_name).map_err(|_| PairingError::InvalidDesktopName)?;
        if certificate_der.is_empty() {
            return Err(PairingError::InvalidFieldLength);
        }

        let mut session_id = [0_u8; SESSION_ID_BYTES];
        let mut secret = [0_u8; INVITATION_SECRET_BYTES];
        let mut challenge = [0_u8; CHALLENGE_BYTES];
        getrandom::fill(&mut session_id).map_err(|_| PairingError::RandomSourceUnavailable)?;
        getrandom::fill(&mut secret).map_err(|_| PairingError::RandomSourceUnavailable)?;
        getrandom::fill(&mut challenge).map_err(|_| PairingError::RandomSourceUnavailable)?;

        Ok(Self {
            version: PAIRING_PROTOCOL_VERSION,
            session_id: encode(&session_id),
            secret: encode(&secret),
            challenge: encode(&challenge),
            certificate_sha256: encode(Sha256::digest(certificate_der).as_slice()),
            endpoint,
            desktop_name: desktop_name.to_owned(),
            expires_at_unix_secs: now_unix_secs
                .checked_add(INVITATION_LIFETIME_SECS)
                .ok_or(PairingError::Expired)?,
        })
    }

    /// Parses and validates an invitation before any network access occurs.
    ///
    /// # Errors
    ///
    /// Returns a typed error for oversized, malformed, expired, unsupported, or
    /// out-of-bounds input.
    pub fn parse(encoded: &str, now_unix_secs: u64) -> Result<Self, PairingError> {
        if encoded.len() > MAX_INVITATION_BYTES {
            return Err(PairingError::InvitationTooLarge);
        }
        let invitation: Self =
            serde_json::from_str(encoded).map_err(|_| PairingError::MalformedInvitation)?;
        invitation.validate(now_unix_secs)?;
        Ok(invitation)
    }

    /// Returns the canonical JSON encoded into the QR. Treat the result as a secret.
    ///
    /// # Errors
    ///
    /// Returns an error only if serialization unexpectedly fails.
    pub fn expose_qr_payload(&self) -> Result<String, PairingError> {
        serde_json::to_string(self).map_err(|_| PairingError::MalformedInvitation)
    }

    #[must_use]
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub fn desktop_name(&self) -> &str {
        &self.desktop_name
    }

    #[must_use]
    pub fn certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }

    fn validate(&self, now_unix_secs: u64) -> Result<(), PairingError> {
        if self.version != PAIRING_PROTOCOL_VERSION {
            return Err(PairingError::UnsupportedVersion);
        }
        validate_endpoint(self.endpoint)?;
        validate_name(&self.desktop_name).map_err(|_| PairingError::InvalidDesktopName)?;
        decode_exact(&self.session_id, SESSION_ID_BYTES)?;
        decode_exact(&self.secret, INVITATION_SECRET_BYTES)?;
        decode_exact(&self.challenge, CHALLENGE_BYTES)?;
        decode_exact(&self.certificate_sha256, CERTIFICATE_DIGEST_BYTES)?;
        if self
            .expires_at_unix_secs
            .saturating_add(MAX_CLOCK_SKEW_SECS)
            <= now_unix_secs
            || self.expires_at_unix_secs
                > now_unix_secs.saturating_add(INVITATION_LIFETIME_SECS + MAX_CLOCK_SKEW_SECS)
        {
            return Err(PairingError::Expired);
        }
        Ok(())
    }
}

impl PairingSession {
    #[must_use]
    pub fn new(invitation: PairingInvitation, now_monotonic_ms: u64) -> Self {
        Self {
            invitation,
            created_monotonic_ms: now_monotonic_ms,
            state: InvitationState::Available,
            verified_claim: None,
            phone_approved: false,
            desktop_approved: false,
        }
    }

    #[must_use]
    pub fn state(&self) -> InvitationState {
        self.state
    }

    #[must_use]
    pub fn invitation(&self) -> &PairingInvitation {
        &self.invitation
    }

    #[must_use]
    pub fn verified_claim(&self) -> Option<&VerifiedClaim> {
        self.verified_claim.as_ref()
    }

    /// Atomically validates and consumes the one-use invitation.
    ///
    /// # Errors
    ///
    /// Rejects expired/replayed sessions, mismatched credentials, malformed
    /// keys, and invalid identity signatures without establishing trust.
    pub fn claim(
        &mut self,
        claim: &PairingClaim,
        now_monotonic_ms: u64,
    ) -> Result<&VerifiedClaim, PairingError> {
        self.expire_if_needed(now_monotonic_ms);
        match self.state {
            InvitationState::Available => {}
            InvitationState::Expired => return Err(PairingError::Expired),
            InvitationState::Rejected => return Err(PairingError::Rejected),
            InvitationState::AwaitingApprovals | InvitationState::Paired => {
                return Err(PairingError::AlreadyClaimed);
            }
        }

        if claim.version != PAIRING_PROTOCOL_VERSION {
            return Err(PairingError::UnsupportedVersion);
        }
        if claim.session_id != self.invitation.session_id {
            return Err(PairingError::WrongSession);
        }
        constant_time_secret_match(&claim.secret, &self.invitation.secret)?;
        validate_name(&claim.phone_name).map_err(|_| PairingError::InvalidPeerName)?;

        let public_key_der =
            decode_bounded(&claim.phone_public_key_der, 1, MAX_PUBLIC_KEY_DER_BYTES)?;
        let phone_nonce = decode_exact(&claim.phone_nonce, PHONE_NONCE_BYTES)?;
        let signature_der = decode_bounded(&claim.signature_der, 1, MAX_SIGNATURE_DER_BYTES)?;
        let verifying_key = VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| PairingError::InvalidPublicKey)?;
        let signature =
            Signature::from_der(&signature_der).map_err(|_| PairingError::InvalidSignature)?;
        let transcript =
            self.invitation
                .claim_transcript(&claim.phone_name, &public_key_der, &phone_nonce)?;
        verifying_key
            .verify(&transcript, &signature)
            .map_err(|_| PairingError::InvalidSignature)?;

        let key_digest = Sha256::digest(&public_key_der);
        let mut phone_key_sha256 = [0_u8; 32];
        phone_key_sha256.copy_from_slice(&key_digest);
        let sas_digest = Sha256::digest(&transcript);
        let sas_number =
            u32::from_be_bytes([sas_digest[0], sas_digest[1], sas_digest[2], sas_digest[3]])
                % 1_000_000;
        self.verified_claim = Some(VerifiedClaim {
            phone_name: claim.phone_name.clone(),
            phone_public_key_der: public_key_der,
            phone_key_sha256,
            sas: format!("{sas_number:06}"),
        });
        self.state = InvitationState::AwaitingApprovals;
        self.verified_claim
            .as_ref()
            .ok_or(PairingError::InvalidSignature)
    }

    /// Records one side's explicit approval and commits only after both approve.
    ///
    /// # Errors
    ///
    /// Returns an error unless a valid claim is waiting for approval.
    pub fn approve(&mut self, side: ApprovalSide) -> Result<InvitationState, PairingError> {
        if self.state != InvitationState::AwaitingApprovals {
            return Err(PairingError::ApprovalOutOfOrder);
        }
        match side {
            ApprovalSide::Phone => self.phone_approved = true,
            ApprovalSide::Desktop => self.desktop_approved = true,
        }
        if self.phone_approved && self.desktop_approved {
            self.state = InvitationState::Paired;
        }
        Ok(self.state)
    }

    pub fn reject(&mut self) {
        if self.state != InvitationState::Paired {
            self.state = InvitationState::Rejected;
            self.verified_claim = None;
            self.phone_approved = false;
            self.desktop_approved = false;
        }
    }

    pub fn expire_if_needed(&mut self, now_monotonic_ms: u64) {
        if self.state != InvitationState::Paired
            && now_monotonic_ms.saturating_sub(self.created_monotonic_ms)
                >= INVITATION_LIFETIME_SECS * 1_000
        {
            self.state = InvitationState::Expired;
            self.verified_claim = None;
            self.phone_approved = false;
            self.desktop_approved = false;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSide {
    Phone,
    Desktop,
}

impl PairingInvitation {
    fn claim_transcript(
        &self,
        phone_name: &str,
        phone_public_key_der: &[u8],
        phone_nonce: &[u8],
    ) -> Result<Vec<u8>, PairingError> {
        let mut transcript = b"OMACAM-PAIR-CLAIM-V1\0".to_vec();
        append_field(&mut transcript, self.session_id.as_bytes())?;
        append_field(&mut transcript, self.challenge.as_bytes())?;
        append_field(&mut transcript, self.certificate_sha256.as_bytes())?;
        append_field(&mut transcript, self.endpoint.to_string().as_bytes())?;
        append_field(&mut transcript, self.desktop_name.as_bytes())?;
        append_field(&mut transcript, phone_name.as_bytes())?;
        append_field(&mut transcript, phone_public_key_der)?;
        append_field(&mut transcript, phone_nonce)?;
        Ok(transcript)
    }
}

fn append_field(target: &mut Vec<u8>, field: &[u8]) -> Result<(), PairingError> {
    let length = u32::try_from(field.len()).map_err(|_| PairingError::InvalidFieldLength)?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(field);
    Ok(())
}

fn validate_endpoint(endpoint: SocketAddr) -> Result<(), PairingError> {
    let ip = endpoint.ip();
    let is_local = match ip {
        std::net::IpAddr::V4(address) => address.is_private() || address.is_link_local(),
        std::net::IpAddr::V6(address) => {
            address.is_unique_local() || address.is_unicast_link_local()
        }
    };
    if endpoint.port() == 0 || !is_local {
        return Err(PairingError::InvalidEndpoint);
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), PairingError> {
    let safe_character = |character: char| {
        character.is_alphanumeric()
            || matches!(character, ' ' | '-' | '_' | '.' | '\'' | '(' | ')' | '+')
    };
    if name.is_empty() || name.len() > MAX_PEER_NAME_BYTES || !name.chars().all(safe_character) {
        return Err(PairingError::InvalidFieldLength);
    }
    Ok(())
}

fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_exact(encoded: &str, expected: usize) -> Result<Vec<u8>, PairingError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| PairingError::InvalidFieldLength)?;
    if decoded.len() != expected {
        return Err(PairingError::InvalidFieldLength);
    }
    Ok(decoded)
}

fn decode_bounded(encoded: &str, minimum: usize, maximum: usize) -> Result<Vec<u8>, PairingError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| PairingError::InvalidFieldLength)?;
    if !(minimum..=maximum).contains(&decoded.len()) {
        return Err(PairingError::InvalidFieldLength);
    }
    Ok(decoded)
}

fn constant_time_secret_match(candidate: &str, expected: &str) -> Result<(), PairingError> {
    let candidate = decode_exact(candidate, INVITATION_SECRET_BYTES)?;
    let expected = decode_exact(expected, INVITATION_SECRET_BYTES)?;
    if bool::from(candidate.ct_eq(&expected)) {
        Ok(())
    } else {
        Err(PairingError::WrongSecret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer as _;
    use p256::pkcs8::EncodePublicKey as _;

    const NOW_UNIX: u64 = 2_000_000_000;
    const NOW_MONOTONIC_MS: u64 = 9_000;

    fn invitation() -> PairingInvitation {
        PairingInvitation::create(
            "192.168.50.10:47123".parse().expect("valid endpoint"),
            "Anand's laptop",
            b"test certificate DER",
            NOW_UNIX,
        )
        .expect("valid invitation")
    }

    fn signed_claim(invitation: &PairingInvitation) -> PairingClaim {
        let signing_key = SigningKey::from_slice(&[7_u8; 32]).expect("valid test key");
        let public_key_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("encode test public key")
            .as_bytes()
            .to_vec();
        let phone_nonce = [9_u8; PHONE_NONCE_BYTES];
        let transcript = invitation
            .claim_transcript("OnePlus Nord 4", &public_key_der, &phone_nonce)
            .expect("valid transcript");
        let signature: Signature = signing_key.sign(&transcript);
        PairingClaim {
            version: PAIRING_PROTOCOL_VERSION,
            session_id: invitation.session_id.clone(),
            secret: invitation.secret.clone(),
            phone_name: "OnePlus Nord 4".to_owned(),
            phone_public_key_der: encode(&public_key_der),
            phone_nonce: encode(&phone_nonce),
            signature_der: encode(signature.to_der().as_bytes()),
        }
    }

    #[test]
    fn invitation_round_trip_is_bounded_and_redacted() {
        let invitation = invitation();
        let payload = invitation.expose_qr_payload().expect("serialize");
        let decoded = PairingInvitation::parse(&payload, NOW_UNIX).expect("parse");

        assert_eq!(decoded.endpoint(), invitation.endpoint());
        assert_eq!(decoded.desktop_name(), "Anand's laptop");
        assert!(payload.len() < MAX_INVITATION_BYTES);
        assert!(!format!("{invitation:?}").contains(&invitation.secret));
    }

    #[test]
    fn malformed_unknown_and_oversized_invitations_fail() {
        assert!(matches!(
            PairingInvitation::parse("not-json", NOW_UNIX),
            Err(PairingError::MalformedInvitation)
        ));
        assert!(matches!(
            PairingInvitation::parse(&"x".repeat(MAX_INVITATION_BYTES + 1), NOW_UNIX),
            Err(PairingError::InvitationTooLarge)
        ));
        let mut value: serde_json::Value =
            serde_json::from_str(&invitation().expose_qr_payload().expect("serialize"))
                .expect("JSON");
        value["unexpected"] = serde_json::json!(true);
        assert!(matches!(
            PairingInvitation::parse(&value.to_string(), NOW_UNIX),
            Err(PairingError::MalformedInvitation)
        ));
    }

    #[test]
    fn public_and_loopback_endpoints_are_rejected() {
        for endpoint in ["8.8.8.8:47123", "127.0.0.1:47123", "[::1]:47123"] {
            assert!(matches!(
                PairingInvitation::create(
                    endpoint.parse().expect("socket address"),
                    "Laptop",
                    b"certificate",
                    NOW_UNIX,
                ),
                Err(PairingError::InvalidEndpoint)
            ));
        }
    }

    #[test]
    fn wall_clock_expiry_is_enforced_after_bounded_skew_tolerance() {
        let payload = invitation().expose_qr_payload().expect("serialize");
        assert!(matches!(
            PairingInvitation::parse(
                &payload,
                NOW_UNIX + INVITATION_LIFETIME_SECS + MAX_CLOCK_SKEW_SECS,
            ),
            Err(PairingError::Expired)
        ));
    }

    #[test]
    fn valid_identity_proof_enters_approval_state() {
        let invitation = invitation();
        let claim = signed_claim(&invitation);
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);

        let verified = session
            .claim(&claim, NOW_MONOTONIC_MS + 1)
            .expect("valid claim");

        assert_eq!(verified.phone_name, "OnePlus Nord 4");
        assert_eq!(verified.sas.len(), 6);
        assert_eq!(session.state(), InvitationState::AwaitingApprovals);
    }

    #[test]
    fn invitation_is_one_use_even_for_the_same_claim() {
        let invitation = invitation();
        let claim = signed_claim(&invitation);
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);
        session
            .claim(&claim, NOW_MONOTONIC_MS)
            .expect("first claim");

        assert_eq!(
            session.claim(&claim, NOW_MONOTONIC_MS),
            Err(PairingError::AlreadyClaimed)
        );
    }

    #[test]
    fn wrong_secret_does_not_consume_invitation() {
        let invitation = invitation();
        let mut wrong = signed_claim(&invitation);
        wrong.secret = encode(&[4_u8; INVITATION_SECRET_BYTES]);
        let valid = signed_claim(&invitation);
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);

        assert_eq!(
            session.claim(&wrong, NOW_MONOTONIC_MS),
            Err(PairingError::WrongSecret)
        );
        assert!(session.claim(&valid, NOW_MONOTONIC_MS).is_ok());
    }

    #[test]
    fn signature_is_bound_to_peer_and_invitation_transcript() {
        let invitation = invitation();
        let mut claim = signed_claim(&invitation);
        claim.phone_name = "Impostor".to_owned();
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);

        assert_eq!(
            session.claim(&claim, NOW_MONOTONIC_MS),
            Err(PairingError::InvalidSignature)
        );
        assert_eq!(session.state(), InvitationState::Available);
    }

    #[test]
    fn monotonic_expiry_rejects_even_if_wall_clock_was_changed() {
        let invitation = invitation();
        let claim = signed_claim(&invitation);
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);

        assert_eq!(
            session.claim(&claim, NOW_MONOTONIC_MS + INVITATION_LIFETIME_SECS * 1_000,),
            Err(PairingError::Expired)
        );
    }

    #[test]
    fn trust_requires_both_approvals() {
        let invitation = invitation();
        let claim = signed_claim(&invitation);
        let mut session = PairingSession::new(invitation, NOW_MONOTONIC_MS);
        session.claim(&claim, NOW_MONOTONIC_MS).expect("claim");

        assert_eq!(
            session
                .approve(ApprovalSide::Phone)
                .expect("phone approval"),
            InvitationState::AwaitingApprovals
        );
        assert_eq!(
            session
                .approve(ApprovalSide::Desktop)
                .expect("desktop approval"),
            InvitationState::Paired
        );
    }
}
