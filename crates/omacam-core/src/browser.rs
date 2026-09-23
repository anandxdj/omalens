//! Browser-provider session protocol: invitation minting, token claiming, and
//! the short authentication string shown on both screens.
//!
//! This module is sans-I/O. It opens no socket, spawns no task, and reads no
//! clock; callers supply monotonic milliseconds from one local clock. The
//! transport is responsible for terminating TLS with the exact certificate
//! whose digest is carried by [`BrowserInvitation`].

use std::fmt;
use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

pub const BROWSER_PROTOCOL_VERSION: u8 = 1;
/// A minted invitation stops being claimable after this long.
pub const INVITATION_LIFETIME_MS: u64 = 120_000;
/// A claimed session that is never confirmed on the desktop is discarded.
pub const CONFIRMATION_LIFETIME_MS: u64 = 120_000;
pub const MAX_INVITATION_BYTES: usize = 1_024;
pub const MAX_TOKEN_BYTES: usize = 64;
pub const MAX_DESKTOP_NAME_BYTES: usize = 64;

const TOKEN_BYTES: usize = 32;
const CERTIFICATE_DIGEST_BYTES: usize = 32;
const SAS_TRANSCRIPT_PREFIX: &[u8] = b"OMACAM-BROWSER-SAS-V1\0";

/// Everything the phone needs, encoded into the QR code.
///
/// The token travels in the URL fragment, which a browser never places in a
/// request line, so it does not reach an access log or a proxy.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserInvitation {
    #[serde(rename = "v")]
    version: u8,
    #[serde(rename = "token")]
    token: String,
    #[serde(rename = "cert")]
    certificate_sha256: String,
    #[serde(rename = "endpoint")]
    endpoint: SocketAddr,
    #[serde(rename = "name")]
    desktop_name: String,
}

impl fmt::Debug for BrowserInvitation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserInvitation")
            .field("version", &self.version)
            .field("token", &"[REDACTED]")
            .field("certificate_sha256", &self.certificate_sha256)
            .field("endpoint", &self.endpoint)
            .field("desktop_name", &self.desktop_name)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserError {
    InvalidVersion,
    InvalidToken,
    InvalidCertificateDigest,
    InvalidEndpoint,
    InvalidDesktopName,
    InvitationExpired,
    AlreadyClaimed,
    NotClaimed,
    NotConfirmed,
    AlreadyConfirmed,
    ConfirmationExpired,
    OversizeInvitation,
    MalformedInvitation,
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidVersion => "browser invitation version is not supported",
            Self::InvalidToken => "browser session token does not match",
            Self::InvalidCertificateDigest => "certificate digest is not a 32-byte SHA-256 value",
            Self::InvalidEndpoint => "endpoint is not a usable private address",
            Self::InvalidDesktopName => "desktop name is empty or too long",
            Self::InvitationExpired => "browser invitation has expired",
            Self::AlreadyClaimed => "browser invitation was already claimed",
            Self::NotClaimed => "browser session has not been claimed",
            Self::NotConfirmed => "browser session is not confirmed on the desktop",
            Self::AlreadyConfirmed => "browser session is already confirmed",
            Self::ConfirmationExpired => "browser session expired before it was confirmed",
            Self::OversizeInvitation => "browser invitation exceeds the size limit",
            Self::MalformedInvitation => "browser invitation is malformed",
        })
    }
}

impl std::error::Error for BrowserError {}

/// Where a session is in the claim/confirm ceremony.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionState {
    /// Minted and shown as a QR code; no phone has opened it yet.
    Offered,
    /// A phone presented the token and is showing the short authentication
    /// string. The desktop has not confirmed.
    AwaitingConfirmation,
    /// The user confirmed on the desktop. Media negotiation may begin.
    Confirmed,
    /// Terminal. A rejected, expired, or superseded session never recovers.
    Closed,
}

impl BrowserInvitation {
    /// Mints an invitation. `token` must be 32 bytes of fresh randomness from
    /// the caller's CSPRNG and `certificate_sha256` the digest of the exact
    /// certificate the HTTPS listener presents.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the certificate digest is the wrong
    /// length, the endpoint is not a usable private address, or the desktop
    /// name is empty or longer than [`MAX_DESKTOP_NAME_BYTES`].
    pub fn create(
        token: &[u8; TOKEN_BYTES],
        certificate_sha256: &[u8],
        endpoint: SocketAddr,
        desktop_name: &str,
    ) -> Result<Self, BrowserError> {
        if certificate_sha256.len() != CERTIFICATE_DIGEST_BYTES {
            return Err(BrowserError::InvalidCertificateDigest);
        }
        validate_endpoint(endpoint)?;
        if desktop_name.is_empty() || desktop_name.len() > MAX_DESKTOP_NAME_BYTES {
            return Err(BrowserError::InvalidDesktopName);
        }
        Ok(Self {
            version: BROWSER_PROTOCOL_VERSION,
            token: URL_SAFE_NO_PAD.encode(token),
            certificate_sha256: URL_SAFE_NO_PAD.encode(certificate_sha256),
            endpoint,
            desktop_name: desktop_name.to_owned(),
        })
    }

    /// Parses an invitation received as a QR payload.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the payload is oversize, malformed, of an
    /// unsupported version, or carries an unusable endpoint or digest.
    pub fn parse(encoded: &str) -> Result<Self, BrowserError> {
        if encoded.len() > MAX_INVITATION_BYTES {
            return Err(BrowserError::OversizeInvitation);
        }
        let invitation: Self =
            serde_json::from_str(encoded).map_err(|_| BrowserError::MalformedInvitation)?;
        if invitation.version != BROWSER_PROTOCOL_VERSION {
            return Err(BrowserError::InvalidVersion);
        }
        if decoded_len(&invitation.certificate_sha256) != Some(CERTIFICATE_DIGEST_BYTES) {
            return Err(BrowserError::InvalidCertificateDigest);
        }
        if decoded_len(&invitation.token) != Some(TOKEN_BYTES) {
            return Err(BrowserError::InvalidToken);
        }
        validate_endpoint(invitation.endpoint)?;
        if invitation.desktop_name.is_empty()
            || invitation.desktop_name.len() > MAX_DESKTOP_NAME_BYTES
        {
            return Err(BrowserError::InvalidDesktopName);
        }
        Ok(invitation)
    }

    /// The URL a phone camera app opens. The token is a fragment, so it stays
    /// out of the request line and out of any server log.
    #[must_use]
    pub fn url(&self) -> String {
        format!("https://{}/#{}", self.endpoint, self.token)
    }

    /// The QR payload. This is the URL, not the JSON: a phone camera app must
    /// recognise it as a link without any application to interpret it.
    #[must_use]
    pub fn qr_payload(&self) -> String {
        self.url()
    }

    #[must_use]
    pub fn certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }

    #[must_use]
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub fn desktop_name(&self) -> &str {
        &self.desktop_name
    }

    /// # Errors
    ///
    /// Returns [`BrowserError::OversizeInvitation`] when the encoded form
    /// exceeds [`MAX_INVITATION_BYTES`].
    pub fn to_json(&self) -> Result<String, BrowserError> {
        let encoded = serde_json::to_string(self).map_err(|_| BrowserError::MalformedInvitation)?;
        if encoded.len() > MAX_INVITATION_BYTES {
            return Err(BrowserError::OversizeInvitation);
        }
        Ok(encoded)
    }
}

/// One browser pairing attempt. Single use: a claimed invitation cannot be
/// claimed again, and a closed session never reopens.
#[derive(Debug)]
pub struct BrowserSession {
    invitation: BrowserInvitation,
    token: [u8; TOKEN_BYTES],
    state: BrowserSessionState,
    created_at_ms: u64,
    claimed_at_ms: Option<u64>,
    sas: Option<String>,
}

impl BrowserSession {
    #[must_use]
    pub fn new(invitation: BrowserInvitation, token: [u8; TOKEN_BYTES], now_ms: u64) -> Self {
        Self {
            invitation,
            token,
            state: BrowserSessionState::Offered,
            created_at_ms: now_ms,
            claimed_at_ms: None,
            sas: None,
        }
    }

    #[must_use]
    pub fn state(&self) -> BrowserSessionState {
        self.state
    }

    #[must_use]
    pub fn invitation(&self) -> &BrowserInvitation {
        &self.invitation
    }

    /// The six digits shown on both screens, available once claimed.
    #[must_use]
    pub fn sas(&self) -> Option<&str> {
        self.sas.as_deref()
    }

    /// Expires an offered or unconfirmed session. Returns `true` when this call
    /// closed it.
    pub fn expire_if_needed(&mut self, now_ms: u64) -> bool {
        let expired = match self.state {
            BrowserSessionState::Offered => {
                now_ms.saturating_sub(self.created_at_ms) >= INVITATION_LIFETIME_MS
            }
            BrowserSessionState::AwaitingConfirmation => self
                .claimed_at_ms
                .is_some_and(|at| now_ms.saturating_sub(at) >= CONFIRMATION_LIFETIME_MS),
            BrowserSessionState::Confirmed | BrowserSessionState::Closed => false,
        };
        if expired {
            self.state = BrowserSessionState::Closed;
        }
        expired
    }

    /// Claims the invitation with the token the phone read from the fragment.
    /// Returns the short authentication string to display on the phone.
    ///
    /// The comparison is constant time, and a mismatch closes the session so a
    /// wrong guess cannot be retried against the same token.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the invitation already expired, was
    /// already claimed, or the presented token does not match.
    pub fn claim(&mut self, presented_token: &str, now_ms: u64) -> Result<&str, BrowserError> {
        if self.expire_if_needed(now_ms) {
            return Err(BrowserError::InvitationExpired);
        }
        match self.state {
            BrowserSessionState::Offered => {}
            BrowserSessionState::Closed => return Err(BrowserError::InvitationExpired),
            BrowserSessionState::AwaitingConfirmation | BrowserSessionState::Confirmed => {
                return Err(BrowserError::AlreadyClaimed);
            }
        }
        if presented_token.len() > MAX_TOKEN_BYTES {
            self.state = BrowserSessionState::Closed;
            return Err(BrowserError::InvalidToken);
        }
        let presented = URL_SAFE_NO_PAD
            .decode(presented_token)
            .map_err(|_| BrowserError::InvalidToken)?;
        if presented.len() != TOKEN_BYTES || presented.ct_eq(&self.token).unwrap_u8() != 1 {
            self.state = BrowserSessionState::Closed;
            return Err(BrowserError::InvalidToken);
        }
        self.state = BrowserSessionState::AwaitingConfirmation;
        self.claimed_at_ms = Some(now_ms);
        let sas = derive_sas(&self.token, self.invitation.certificate_sha256());
        Ok(self.sas.insert(sas))
    }

    /// Records the desktop-side confirmation. Capture may only be negotiated
    /// after this returns `Ok`.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the session was never claimed, already
    /// confirmed, or expired while waiting.
    pub fn confirm(&mut self, now_ms: u64) -> Result<(), BrowserError> {
        if self.expire_if_needed(now_ms) {
            return Err(BrowserError::ConfirmationExpired);
        }
        match self.state {
            BrowserSessionState::AwaitingConfirmation => {
                self.state = BrowserSessionState::Confirmed;
                Ok(())
            }
            BrowserSessionState::Offered => Err(BrowserError::NotClaimed),
            BrowserSessionState::Confirmed => Err(BrowserError::AlreadyConfirmed),
            BrowserSessionState::Closed => Err(BrowserError::ConfirmationExpired),
        }
    }

    /// Closes the session. Idempotent and terminal.
    pub fn close(&mut self) {
        self.state = BrowserSessionState::Closed;
    }

    /// Whether media negotiation is permitted right now.
    #[must_use]
    pub fn may_negotiate(&self) -> bool {
        self.state == BrowserSessionState::Confirmed
    }
}

/// Six digits bound to both the session token and the certificate the phone was
/// served, so a listener that answered the address first produces different
/// digits from the ones the desktop shows.
fn derive_sas(token: &[u8; TOKEN_BYTES], certificate_sha256: &str) -> String {
    let mut transcript = Vec::with_capacity(96);
    transcript.extend_from_slice(SAS_TRANSCRIPT_PREFIX);
    append_field(&mut transcript, token);
    append_field(&mut transcript, certificate_sha256.as_bytes());
    let digest = Sha256::digest(&transcript);
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{value:06}")
}

/// Length-prefixed so no field boundary can be shifted into another field.
fn append_field(transcript: &mut Vec<u8>, field: &[u8]) {
    let length = u32::try_from(field.len()).unwrap_or(u32::MAX);
    transcript.extend_from_slice(&length.to_be_bytes());
    transcript.extend_from_slice(field);
}

fn decoded_len(encoded: &str) -> Option<usize> {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .map(|bytes| bytes.len())
}

/// The browser provider only ever advertises an address the phone can reach on
/// the same network. A public address would invite the internet in, and a
/// loopback address is unreachable from the phone.
fn validate_endpoint(endpoint: SocketAddr) -> Result<(), BrowserError> {
    if endpoint.port() == 0 {
        return Err(BrowserError::InvalidEndpoint);
    }
    let private = match endpoint.ip() {
        std::net::IpAddr::V4(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_broadcast()
                && (address.is_private() || address.is_link_local())
        }
        std::net::IpAddr::V6(address) => {
            let segments = address.segments();
            let unique_local = segments[0] & 0xfe00 == 0xfc00;
            let link_local = segments[0] & 0xffc0 == 0xfe80;
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && (unique_local || link_local)
        }
    };
    if private {
        Ok(())
    } else {
        Err(BrowserError::InvalidEndpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: [u8; CERTIFICATE_DIGEST_BYTES] = [7u8; CERTIFICATE_DIGEST_BYTES];

    fn endpoint() -> SocketAddr {
        "192.168.1.5:8443".parse().expect("endpoint")
    }

    fn session(now_ms: u64) -> (BrowserSession, String) {
        let token = [3u8; TOKEN_BYTES];
        let invitation =
            BrowserInvitation::create(&token, &DIGEST, endpoint(), "desk").expect("invitation");
        let encoded = URL_SAFE_NO_PAD.encode(token);
        (BrowserSession::new(invitation, token, now_ms), encoded)
    }

    #[test]
    fn url_carries_the_token_only_in_the_fragment() {
        let (session, encoded) = session(0);
        let url = session.invitation().url();
        let (before_fragment, fragment) = url.split_once('#').expect("fragment");
        assert_eq!(fragment, encoded);
        assert!(!before_fragment.contains(&encoded));
        assert_eq!(before_fragment, "https://192.168.1.5:8443/");
    }

    #[test]
    fn claim_then_confirm_is_the_only_path_to_negotiation() {
        let (mut session, encoded) = session(0);
        assert!(!session.may_negotiate());
        assert_eq!(session.confirm(0), Err(BrowserError::NotClaimed));

        let sas = session.claim(&encoded, 10).expect("claim").to_owned();
        assert_eq!(sas.len(), 6);
        assert!(sas.chars().all(|value| value.is_ascii_digit()));
        assert_eq!(session.state(), BrowserSessionState::AwaitingConfirmation);
        assert!(!session.may_negotiate());

        session.confirm(20).expect("confirm");
        assert!(session.may_negotiate());
        assert_eq!(session.confirm(30), Err(BrowserError::AlreadyConfirmed));
    }

    #[test]
    fn a_wrong_token_closes_the_session_instead_of_allowing_a_retry() {
        let (mut session, correct) = session(0);
        let wrong = URL_SAFE_NO_PAD.encode([9u8; TOKEN_BYTES]);
        assert_eq!(session.claim(&wrong, 1), Err(BrowserError::InvalidToken));
        assert_eq!(session.state(), BrowserSessionState::Closed);

        assert_eq!(
            session.claim(&correct, 2),
            Err(BrowserError::InvitationExpired)
        );
        assert!(!session.may_negotiate());
    }

    #[test]
    fn an_invitation_cannot_be_claimed_twice() {
        let (mut session, encoded) = session(0);
        session.claim(&encoded, 1).expect("first claim");
        assert_eq!(
            session.claim(&encoded, 2),
            Err(BrowserError::AlreadyClaimed)
        );
    }

    #[test]
    fn offered_and_unconfirmed_sessions_both_expire() {
        let (mut offered, _) = session(0);
        assert!(offered.expire_if_needed(INVITATION_LIFETIME_MS));
        assert_eq!(offered.state(), BrowserSessionState::Closed);

        let (mut claimed, encoded) = session(0);
        claimed.claim(&encoded, 1).expect("claim");
        assert!(!claimed.expire_if_needed(CONFIRMATION_LIFETIME_MS));
        assert!(claimed.expire_if_needed(1 + CONFIRMATION_LIFETIME_MS));
        assert_eq!(
            claimed.confirm(2 + CONFIRMATION_LIFETIME_MS),
            Err(BrowserError::ConfirmationExpired)
        );
    }

    #[test]
    fn the_short_authentication_string_binds_the_served_certificate() {
        let token = [3u8; TOKEN_BYTES];
        let first = derive_sas(&token, &URL_SAFE_NO_PAD.encode(DIGEST));
        let second = derive_sas(&token, &URL_SAFE_NO_PAD.encode([8u8; 32]));
        assert_ne!(first, second);
        assert_eq!(first, derive_sas(&token, &URL_SAFE_NO_PAD.encode(DIGEST)));
    }

    #[test]
    fn invitations_round_trip_and_reject_bad_input() {
        let (session, _) = session(0);
        let encoded = session.invitation().to_json().expect("encode");
        let parsed = BrowserInvitation::parse(&encoded).expect("parse");
        assert_eq!(parsed.endpoint(), endpoint());
        assert_eq!(parsed.desktop_name(), "desk");

        assert_eq!(
            BrowserInvitation::parse("{").unwrap_err(),
            BrowserError::MalformedInvitation
        );
        assert_eq!(
            BrowserInvitation::parse(&"x".repeat(MAX_INVITATION_BYTES + 1)).unwrap_err(),
            BrowserError::OversizeInvitation
        );
    }

    #[test]
    fn public_and_loopback_endpoints_are_refused() {
        let token = [1u8; TOKEN_BYTES];
        for address in ["8.8.8.8:8443", "127.0.0.1:8443", "192.168.1.5:0"] {
            assert_eq!(
                BrowserInvitation::create(
                    &token,
                    &DIGEST,
                    address.parse().expect("address"),
                    "desk"
                )
                .unwrap_err(),
                BrowserError::InvalidEndpoint,
                "{address} must be refused"
            );
        }
        assert!(
            BrowserInvitation::create(&token, &DIGEST, "10.0.0.4:8443".parse().unwrap(), "desk")
                .is_ok()
        );
        for address in ["[2001:4860:4860::8888]:8443", "[::1]:8443"] {
            assert_eq!(
                BrowserInvitation::create(&token, &DIGEST, address.parse().unwrap(), "desk")
                    .unwrap_err(),
                BrowserError::InvalidEndpoint,
                "{address} must be refused"
            );
        }
        assert!(
            BrowserInvitation::create(&token, &DIGEST, "[fd12::5]:8443".parse().unwrap(), "desk")
                .is_ok()
        );
    }

    #[test]
    fn a_short_certificate_digest_is_refused() {
        assert_eq!(
            BrowserInvitation::create(&[1u8; TOKEN_BYTES], &[0u8; 16], endpoint(), "desk")
                .unwrap_err(),
            BrowserError::InvalidCertificateDigest
        );
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let (session, encoded) = session(0);
        let rendered = format!("{:?}", session.invitation());
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains(&encoded));
    }
}
