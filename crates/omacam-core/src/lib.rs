//! Safety-critical session policy for `OmaCam`.
//!
//! This crate deliberately contains no transport, UI, or operating-system code.
//! Callers provide monotonic millisecond timestamps from one local clock.

pub mod pairing;

pub const DEFAULT_CAPTURE_LEASE_MS: u64 = 10_000;
pub const MAX_FRAME_AGE_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    Unpaired,
    Trusted,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Offline,
    Online,
    Recovering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Idle,
    AwaitingConsent,
    Starting,
    Streaming,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputState {
    Missing,
    ReadyNeutral,
    ReadyLive,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    PeerNotTrusted,
    PeerOffline,
    CaptureBusy,
    ConsentNotPending,
    SessionNotStarting,
    SessionNotStreaming,
    StaleGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub trust: TrustState,
    pub connection: ConnectionState,
    pub capture: CaptureState,
    pub output: OutputState,
    pub generation: u64,
    pub capture_armed: bool,
}

#[derive(Debug)]
pub struct SessionPolicy {
    trust: TrustState,
    connection: ConnectionState,
    capture: CaptureState,
    output: OutputState,
    generation: u64,
    lease_expires_at_ms: Option<u64>,
    last_frame_at_ms: Option<u64>,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            trust: TrustState::Unpaired,
            connection: ConnectionState::Offline,
            capture: CaptureState::Idle,
            output: OutputState::Missing,
            generation: 0,
            lease_expires_at_ms: None,
            last_frame_at_ms: None,
        }
    }
}

impl SessionPolicy {
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            trust: self.trust,
            connection: self.connection,
            capture: self.capture,
            output: self.output,
            generation: self.generation,
            capture_armed: self.lease_expires_at_ms.is_some(),
        }
    }

    pub fn set_output_ready(&mut self) {
        self.output = OutputState::ReadyNeutral;
    }

    pub fn set_output_failed(&mut self) {
        self.output = OutputState::Failed;
    }

    pub fn trust_peer(&mut self) {
        self.trust = TrustState::Trusted;
    }

    /// Marks the selected peer online after its stored identity is authenticated.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::PeerNotTrusted`] when pairing has not established trust.
    pub fn authenticate_connection(&mut self) -> Result<(), PolicyError> {
        if self.trust != TrustState::Trusted {
            return Err(PolicyError::PeerNotTrusted);
        }
        self.connection = ConnectionState::Online;
        Ok(())
    }

    /// Begins a request which still requires explicit consent on the phone.
    ///
    /// # Errors
    ///
    /// Returns an error when the peer is untrusted or offline, or another capture
    /// request/session already owns the state machine.
    pub fn request_start(&mut self) -> Result<(), PolicyError> {
        if self.trust != TrustState::Trusted {
            return Err(PolicyError::PeerNotTrusted);
        }
        if self.connection != ConnectionState::Online {
            return Err(PolicyError::PeerOffline);
        }
        if self.capture != CaptureState::Idle {
            return Err(PolicyError::CaptureBusy);
        }
        self.capture = CaptureState::AwaitingConsent;
        Ok(())
    }

    /// Arms a new capture generation after explicit phone consent.
    ///
    /// # Errors
    ///
    /// Returns an error when no consent request is pending or when the trusted
    /// connection disappeared before approval.
    pub fn grant_consent(&mut self, now_ms: u64) -> Result<u64, PolicyError> {
        if self.capture != CaptureState::AwaitingConsent {
            return Err(PolicyError::ConsentNotPending);
        }
        if self.trust != TrustState::Trusted {
            return Err(PolicyError::PeerNotTrusted);
        }
        if self.connection != ConnectionState::Online {
            return Err(PolicyError::PeerOffline);
        }
        self.generation = self.generation.wrapping_add(1);
        self.capture = CaptureState::Starting;
        self.lease_expires_at_ms = Some(now_ms.saturating_add(DEFAULT_CAPTURE_LEASE_MS));
        self.last_frame_at_ms = None;
        self.force_neutral();
        Ok(self.generation)
    }

    /// Confirms that capture resources for the current generation are running.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation or a session outside `Starting`.
    pub fn capture_started(&mut self, generation: u64) -> Result<(), PolicyError> {
        self.require_generation(generation)?;
        if self.capture != CaptureState::Starting {
            return Err(PolicyError::SessionNotStarting);
        }
        self.capture = CaptureState::Streaming;
        Ok(())
    }

    /// Refreshes the authorization lease for the current capture generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation or a session which is not armed.
    pub fn refresh_lease(&mut self, generation: u64, now_ms: u64) -> Result<(), PolicyError> {
        self.require_generation(generation)?;
        if !matches!(
            self.capture,
            CaptureState::Starting | CaptureState::Streaming
        ) {
            return Err(PolicyError::SessionNotStreaming);
        }
        self.lease_expires_at_ms = Some(now_ms.saturating_add(DEFAULT_CAPTURE_LEASE_MS));
        Ok(())
    }

    /// Marks a fresh frame from the current streaming generation as publishable.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale generation or a session which is not streaming.
    pub fn accept_frame(&mut self, generation: u64, now_ms: u64) -> Result<(), PolicyError> {
        self.require_generation(generation)?;
        if self.capture != CaptureState::Streaming {
            return Err(PolicyError::SessionNotStreaming);
        }
        self.last_frame_at_ms = Some(now_ms);
        if self.output == OutputState::ReadyNeutral {
            self.output = OutputState::ReadyLive;
        }
        Ok(())
    }

    pub fn connection_lost(&mut self) {
        self.connection = ConnectionState::Recovering;
        self.last_frame_at_ms = None;
        self.force_neutral();
    }

    pub fn tick(&mut self, now_ms: u64) {
        if self
            .lease_expires_at_ms
            .is_some_and(|expiry| now_ms >= expiry)
        {
            self.stop();
            return;
        }

        if self
            .last_frame_at_ms
            .is_some_and(|last_frame| now_ms.saturating_sub(last_frame) >= MAX_FRAME_AGE_MS)
        {
            self.last_frame_at_ms = None;
            self.force_neutral();
        }
    }

    pub fn stop(&mut self) {
        let had_session = self.capture != CaptureState::Idle
            || self.lease_expires_at_ms.is_some()
            || self.last_frame_at_ms.is_some();
        if had_session {
            self.generation = self.generation.wrapping_add(1);
        }
        self.capture = CaptureState::Idle;
        self.lease_expires_at_ms = None;
        self.last_frame_at_ms = None;
        self.force_neutral();
    }

    pub fn forget_peer(&mut self) {
        self.stop();
        self.trust = TrustState::Revoked;
        self.connection = ConnectionState::Offline;
    }

    fn require_generation(&self, generation: u64) -> Result<(), PolicyError> {
        if self.generation == generation {
            Ok(())
        } else {
            Err(PolicyError::StaleGeneration)
        }
    }

    fn force_neutral(&mut self) {
        if self.output == OutputState::ReadyLive {
            self.output = OutputState::ReadyNeutral;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streaming_policy(now_ms: u64) -> (SessionPolicy, u64) {
        let mut policy = SessionPolicy::default();
        policy.set_output_ready();
        policy.trust_peer();
        policy.authenticate_connection().unwrap();
        policy.request_start().unwrap();
        let generation = policy.grant_consent(now_ms).unwrap();
        policy.capture_started(generation).unwrap();
        (policy, generation)
    }

    #[test]
    fn trust_and_connection_do_not_start_capture() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().unwrap();

        assert_eq!(policy.snapshot().capture, CaptureState::Idle);
        assert!(!policy.snapshot().capture_armed);
    }

    #[test]
    fn consent_is_required_before_a_session_is_armed() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().unwrap();
        policy.request_start().unwrap();

        assert_eq!(policy.snapshot().capture, CaptureState::AwaitingConsent);
        assert!(!policy.snapshot().capture_armed);
    }

    #[test]
    fn old_frames_cannot_restore_output_after_stop() {
        let (mut policy, generation) = streaming_policy(1_000);
        policy.accept_frame(generation, 1_001).unwrap();
        assert_eq!(policy.snapshot().output, OutputState::ReadyLive);

        policy.stop();

        assert_eq!(policy.snapshot().output, OutputState::ReadyNeutral);
        assert_eq!(
            policy.accept_frame(generation, 1_002),
            Err(PolicyError::StaleGeneration)
        );
        assert_eq!(policy.snapshot().output, OutputState::ReadyNeutral);
    }

    #[test]
    fn stale_frame_forces_neutral_output() {
        let (mut policy, generation) = streaming_policy(2_000);
        policy.accept_frame(generation, 2_100).unwrap();

        policy.tick(2_100 + MAX_FRAME_AGE_MS);

        assert_eq!(policy.snapshot().capture, CaptureState::Streaming);
        assert_eq!(policy.snapshot().output, OutputState::ReadyNeutral);
    }

    #[test]
    fn expired_lease_disarms_capture() {
        let (mut policy, _) = streaming_policy(4_000);

        policy.tick(4_000 + DEFAULT_CAPTURE_LEASE_MS);

        assert_eq!(policy.snapshot().capture, CaptureState::Idle);
        assert!(!policy.snapshot().capture_armed);
        assert_eq!(policy.snapshot().output, OutputState::ReadyNeutral);
    }

    #[test]
    fn connection_loss_clears_live_output_without_revoking_lease() {
        let (mut policy, generation) = streaming_policy(8_000);
        policy.accept_frame(generation, 8_001).unwrap();

        policy.connection_lost();

        assert_eq!(policy.snapshot().connection, ConnectionState::Recovering);
        assert_eq!(policy.snapshot().output, OutputState::ReadyNeutral);
        assert!(policy.snapshot().capture_armed);
    }

    #[test]
    fn forget_revokes_and_invalidates_the_active_session() {
        let (mut policy, generation) = streaming_policy(10_000);

        policy.forget_peer();

        let snapshot = policy.snapshot();
        assert_eq!(snapshot.trust, TrustState::Revoked);
        assert_eq!(snapshot.connection, ConnectionState::Offline);
        assert_eq!(snapshot.capture, CaptureState::Idle);
        assert_eq!(
            policy.refresh_lease(generation, 10_001),
            Err(PolicyError::StaleGeneration)
        );
    }

    #[test]
    fn second_start_cannot_replace_an_active_session() {
        let (mut policy, generation) = streaming_policy(12_000);

        assert_eq!(policy.request_start(), Err(PolicyError::CaptureBusy));
        assert_eq!(policy.snapshot().capture, CaptureState::Streaming);
        assert_eq!(policy.snapshot().generation, generation);
    }

    #[test]
    fn consent_cannot_arm_capture_after_connection_loss() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().unwrap();
        policy.request_start().unwrap();
        policy.connection_lost();

        assert_eq!(policy.grant_consent(14_000), Err(PolicyError::PeerOffline));
        assert!(!policy.snapshot().capture_armed);
    }

    #[test]
    fn repeated_stop_is_observably_idempotent() {
        let (mut policy, _) = streaming_policy(16_000);
        policy.stop();
        let after_first_stop = policy.snapshot();

        policy.stop();

        assert_eq!(policy.snapshot(), after_first_stop);
    }
}
