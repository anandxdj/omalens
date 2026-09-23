use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use omacam_core::camera::{AppliedCameraState, CameraCapabilities, RequestedControls};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::net::SocketAddr;
use std::path::PathBuf;

pub(super) const PHONE_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(8);
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_PIXELS: u32 = 3840 * 2160;
const RECENT_COMMAND_ID_LIMIT: usize = 256;
const LIVE_FORMAT_CHANGE_UNAVAILABLE: &str =
    "resolution and frame rate changes are unavailable during a live stream";

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) phone_token: String,
    pub(super) desktop_token: String,
    pub(super) media_endpoint: SocketAddr,
    pub(super) output_device: PathBuf,
    pub(super) claimed: Arc<AtomicBool>,
    pub(super) session: Arc<Mutex<CameraSession>>,
    state_poll_at: Arc<Mutex<Option<Instant>>>,
    preview_poll_at: Arc<Mutex<Option<Instant>>>,
}

impl AppState {
    pub(super) fn new(
        phone_token: String,
        desktop_token: String,
        media_endpoint: SocketAddr,
        output_device: PathBuf,
        session_id: String,
        now: Instant,
    ) -> Self {
        Self {
            phone_token,
            desktop_token,
            media_endpoint,
            output_device,
            claimed: Arc::new(AtomicBool::new(false)),
            session: Arc::new(Mutex::new(CameraSession::new(session_id, now))),
            state_poll_at: Arc::new(Mutex::new(None)),
            preview_poll_at: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn allow_poll(&self, preview: bool, now: Instant) -> bool {
        let (lock, minimum_interval) = if preview {
            (&self.preview_poll_at, Duration::from_millis(200))
        } else {
            (&self.state_poll_at, Duration::from_millis(100))
        };
        let mut last = lock.lock().unwrap();
        if last.is_some_and(|at| now.saturating_duration_since(at) < minimum_interval) {
            return false;
        }
        *last = Some(now);
        true
    }
}

#[derive(Clone, Copy)]
pub(super) enum Role {
    Phone,
    Desktop,
}

pub(super) fn authorized(headers: &HeaderMap, state: &AppState, role: Role) -> bool {
    let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    let expected = match role {
        Role::Phone => &state.phone_token,
        Role::Desktop => &state.desktop_token,
    };
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct VideoFormat {
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) fps: u8,
}

impl VideoFormat {
    pub(super) fn validate(self) -> Result<Self, &'static str> {
        let pixels = u32::from(self.width) * u32::from(self.height);
        if !(320..=4096).contains(&self.width)
            || !(240..=4096).contains(&self.height)
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || pixels > MAX_OUTPUT_PIXELS
            || !(15..=60).contains(&self.fps)
        {
            return Err("camera format must be even-sized, at most 4K, and 15–60 fps");
        }
        Ok(self)
    }

    pub(super) fn matches(self, applied: &AppliedCameraState) -> bool {
        u32::from(self.width) == applied.width
            && u32::from(self.height) == applied.height
            && (f64::from(self.fps) - applied.fps).abs() <= 0.5
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserOffer {
    pub(super) offer: str0m::change::SdpOffer,
    pub(super) format: VideoFormat,
    pub(super) capabilities: Option<CameraCapabilities>,
    pub(super) applied: Option<AppliedCameraState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CameraCommand {
    pub(super) session_id: String,
    pub(super) command_id: String,
    pub(super) expected_revision: u64,
    pub(super) controls: RequestedControls,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PhoneHeartbeat {
    pub(super) session_id: String,
    pub(super) capabilities: Option<CameraCapabilities>,
    pub(super) applied: Option<AppliedCameraState>,
    pub(super) command_id: Option<String>,
    pub(super) error: Option<String>,
    pub(super) stats: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionStats {
    pub(super) phone: Option<Value>,
    pub(super) received_frames: u64,
    pub(super) received_keyframes: u64,
    pub(super) received_bytes: u64,
    pub(super) dropped_frames: u64,
    pub(super) written_frames: u64,
    pub(super) preview_frames: u64,
    pub(super) last_frame_age_ms: Option<u64>,
    #[serde(skip)]
    last_frame_at: Option<Instant>,
}

impl SessionStats {
    pub(super) fn note_received(&mut self, keyframe: bool, bytes: usize) {
        self.received_frames = self.received_frames.saturating_add(1);
        self.received_keyframes = self.received_keyframes.saturating_add(u64::from(keyframe));
        self.received_bytes = self
            .received_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        self.last_frame_at = Some(Instant::now());
        self.last_frame_age_ms = Some(0);
    }

    pub(super) fn note_dropped(&mut self, count: u64) {
        self.dropped_frames = self.dropped_frames.saturating_add(count);
    }

    pub(super) fn note_written(&mut self) {
        self.written_frames = self.written_frames.saturating_add(1);
    }

    pub(super) fn note_preview(&mut self) {
        self.preview_frames = self.preview_frames.saturating_add(1);
    }

    pub(super) fn refresh_age(&mut self) {
        self.last_frame_age_ms = self
            .last_frame_at
            .map(|at| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
    }
}

#[derive(Clone)]
pub(super) struct PreviewFrame {
    pub(super) captured_at: Instant,
    pub(super) jpeg: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CameraSession {
    pub(super) session_id: String,
    pub(super) revision: u64,
    pub(super) status: String,
    pub(super) capabilities: CameraCapabilities,
    pub(super) applied: Option<AppliedCameraState>,
    pub(super) pending: Option<CameraCommand>,
    pub(super) last_command_id: Option<String>,
    pub(super) last_command_outcome: Option<CommandOutcome>,
    pub(super) error: Option<String>,
    pub(super) stats: SessionStats,
    #[serde(skip)]
    pub(super) heartbeat: Instant,
    #[serde(skip)]
    pub(super) cancel: Arc<AtomicBool>,
    #[serde(skip)]
    pub(super) preview: Option<PreviewFrame>,
    #[serde(skip)]
    pending_since: Option<Instant>,
    #[serde(skip)]
    output_format: Option<VideoFormat>,
    #[serde(skip)]
    seen_command_ids: VecDeque<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum CommandOutcomeStatus {
    Applied,
    Adjusted,
    Rejected,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommandOutcome {
    pub(super) command_id: String,
    pub(super) status: CommandOutcomeStatus,
    pub(super) message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CommandError {
    NotLive,
    StaleRevision,
    Pending,
    InvalidId,
    ReusedId,
    InvalidControls(&'static str),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum HeartbeatError {
    WrongSession,
    NotLive,
    StaleAcknowledgement,
    MissingAppliedState,
    InvalidCapabilities(&'static str),
    InvalidAppliedState(&'static str),
}

impl CameraSession {
    fn new(session_id: String, now: Instant) -> Self {
        Self {
            session_id,
            revision: 0,
            status: "waiting".into(),
            capabilities: CameraCapabilities::default(),
            applied: None,
            pending: None,
            last_command_id: None,
            last_command_outcome: None,
            error: None,
            stats: SessionStats::default(),
            heartbeat: now,
            cancel: Arc::new(AtomicBool::new(false)),
            preview: None,
            pending_since: None,
            output_format: None,
            seen_command_ids: VecDeque::new(),
        }
    }

    pub(super) fn activate(
        &mut self,
        capabilities: CameraCapabilities,
        applied: AppliedCameraState,
        output_format: VideoFormat,
        now: Instant,
    ) {
        self.capabilities = capabilities;
        self.applied = Some(applied);
        self.output_format = Some(output_format);
        self.heartbeat = now;
        self.status = "live".into();
        self.error = None;
        self.revision = self.revision.saturating_add(1);
    }

    pub(super) fn stop(&mut self, reason: &str) {
        if self.status == "stopped" {
            return;
        }
        self.cancel.store(true, Ordering::Release);
        self.pending = None;
        self.pending_since = None;
        self.preview = None;
        self.status = "stopped".into();
        self.error = Some(reason.into());
        self.revision = self.revision.saturating_add(1);
    }

    pub(super) fn expire_heartbeat_if_needed(&mut self, now: Instant) -> bool {
        if self.status == "live"
            && now.saturating_duration_since(self.heartbeat) > PHONE_HEARTBEAT_TIMEOUT
        {
            self.stop("Phone heartbeat expired");
            return true;
        }
        false
    }

    pub(super) fn enqueue_command(
        &mut self,
        command: CameraCommand,
        now: Instant,
    ) -> Result<(), CommandError> {
        if command.session_id != self.session_id {
            return Err(CommandError::StaleRevision);
        }
        if command.controls.stop == Some(true) {
            self.stop("Stopped from desktop");
            return Ok(());
        }
        if self.status != "live" {
            return Err(CommandError::NotLive);
        }
        if command.expected_revision != self.revision {
            return Err(CommandError::StaleRevision);
        }
        if self.pending.is_some() {
            return Err(CommandError::Pending);
        }
        if command.command_id.is_empty()
            || command.command_id.len() > 128
            || !command
                .command_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(CommandError::InvalidId);
        }
        if self.seen_command_ids.contains(&command.command_id) {
            return Err(CommandError::ReusedId);
        }
        if self.output_format.is_some_and(|fixed| {
            command
                .controls
                .width
                .is_some_and(|width| width != u32::from(fixed.width))
                || command
                    .controls
                    .height
                    .is_some_and(|height| height != u32::from(fixed.height))
                || command
                    .controls
                    .fps
                    .is_some_and(|fps| (fps - f64::from(fixed.fps)).abs() > 0.5)
        }) {
            return Err(CommandError::InvalidControls(
                LIVE_FORMAT_CHANGE_UNAVAILABLE,
            ));
        }
        command
            .controls
            .validate(&self.capabilities)
            .map_err(CommandError::InvalidControls)?;
        let command_id = command.command_id.clone();
        self.pending = Some(command);
        self.pending_since = Some(now);
        self.seen_command_ids.push_back(command_id);
        if self.seen_command_ids.len() > RECENT_COMMAND_ID_LIMIT {
            self.seen_command_ids.pop_front();
        }
        self.last_command_outcome = None;
        self.error = None;
        Ok(())
    }

    pub(super) fn expire_pending_if_needed(&mut self, now: Instant) -> bool {
        if self.pending.is_some()
            && self
                .pending_since
                .is_some_and(|since| now.saturating_duration_since(since) >= COMMAND_TIMEOUT)
        {
            let command_id = self
                .pending
                .as_ref()
                .map(|command| command.command_id.clone())
                .unwrap_or_default();
            self.pending = None;
            self.pending_since = None;
            let message = "Phone did not acknowledge the camera command before its deadline";
            self.last_command_id = Some(command_id.clone());
            self.last_command_outcome = Some(CommandOutcome {
                command_id,
                status: CommandOutcomeStatus::Rejected,
                message: Some(message.into()),
            });
            self.error = Some(message.into());
            self.revision = self.revision.saturating_add(1);
            return true;
        }
        false
    }

    pub(super) fn accept_heartbeat(
        &mut self,
        update: PhoneHeartbeat,
        now: Instant,
    ) -> Result<(), HeartbeatError> {
        if update.session_id != self.session_id {
            return Err(HeartbeatError::WrongSession);
        }
        if self.status != "live" {
            return Err(HeartbeatError::NotLive);
        }
        self.expire_pending_if_needed(now);

        let next_capabilities = update
            .capabilities
            .unwrap_or_else(|| self.capabilities.clone());
        next_capabilities
            .validate()
            .map_err(HeartbeatError::InvalidCapabilities)?;
        let mut changed = !same_json(&self.capabilities, &next_capabilities);
        let current_command = if let Some(command_id) = update.command_id.as_deref() {
            let Some(pending) = self
                .pending
                .as_ref()
                .filter(|pending| pending.command_id == command_id)
            else {
                return Err(HeartbeatError::StaleAcknowledgement);
            };
            Some(pending.clone())
        } else {
            None
        };

        let mut command_outcome = None;
        let mut command_error = None;
        if let Some(applied) = update.applied {
            applied
                .validate(&next_capabilities)
                .map_err(HeartbeatError::InvalidAppliedState)?;
            if let Some(command) = &current_command {
                let phone_error = update.error.as_deref().filter(|error| !error.is_empty());
                let reflected = command.controls.is_reflected_by(&applied);
                let (status, message) = if let Some(error) = phone_error {
                    (CommandOutcomeStatus::Rejected, Some(error.to_owned()))
                } else if reflected {
                    (CommandOutcomeStatus::Applied, None)
                } else {
                    (
                        CommandOutcomeStatus::Adjusted,
                        Some(format!(
                            "Camera applied different settings than requested; actual format is {}x{} at {:.1} fps",
                            applied.width, applied.height, applied.fps
                        )),
                    )
                };
                command_error.clone_from(&message);
                command_outcome = Some(CommandOutcome {
                    command_id: command.command_id.clone(),
                    status,
                    message,
                });
            }
            changed |= !same_json(&self.applied, &Some(applied.clone()));
            self.applied = Some(applied);
        } else if current_command.is_some() {
            return Err(HeartbeatError::MissingAppliedState);
        }

        self.capabilities = next_capabilities;
        if let Some(command) = current_command {
            self.pending = None;
            self.pending_since = None;
            self.last_command_id = Some(command.command_id);
            self.last_command_outcome = command_outcome;
            self.error = command_error;
            changed = true;
        } else if let Some(error) = update.error {
            self.error = Some(error);
        }
        if let Some(stats) = update.stats {
            self.stats.phone = Some(stats);
        }
        self.heartbeat = now;
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn observe_received_frame(&mut self, keyframe: bool, bytes: usize) {
        self.stats.note_received(keyframe, bytes);
    }

    pub(super) fn observe_dropped_frames(&mut self, count: u64) {
        self.stats.note_dropped(count);
    }

    pub(super) fn observe_written_frame(&mut self) {
        self.stats.note_written();
    }

    pub(super) fn observe_preview_frame(&mut self) {
        self.stats.note_preview();
    }

    pub(super) fn refresh_snapshot(&mut self) -> Value {
        self.stats.refresh_age();
        serde_json::to_value(&*self).expect("camera session is serializable")
    }
}

fn same_json(left: &impl Serialize, right: &impl Serialize) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

pub(super) fn binding_for_token(
    token: &[u8],
) -> Result<omacam_core::media::MediaBinding, getrandom::Error> {
    use omacam_core::media::{ControlConnectionId, MediaBinding, MediaSessionId, PeerIdentity};
    let mut connection = [0_u8; 16];
    let mut session = [0_u8; 16];
    getrandom::fill(&mut connection)?;
    getrandom::fill(&mut session)?;
    Ok(MediaBinding {
        peer_identity: PeerIdentity::new(Sha256::digest(token).into()),
        control_connection_id: ControlConnectionId::new(connection),
        media_session_id: MediaSessionId::new(session),
        generation: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacam_core::camera::{Camera, CameraMode};

    fn live_session() -> CameraSession {
        let now = Instant::now();
        let caps = CameraCapabilities {
            cameras: vec![Camera {
                id: "rear".into(),
                label: "Rear".into(),
                facing: "environment".into(),
                modes: vec![
                    CameraMode {
                        width: 1280,
                        height: 720,
                        fps: 30.0,
                    },
                    CameraMode {
                        width: 1920,
                        height: 1080,
                        fps: 30.0,
                    },
                    CameraMode {
                        width: 1280,
                        height: 720,
                        fps: 60.0,
                    },
                ],
                frame_rates: vec![30.0, 60.0],
            }],
            ..Default::default()
        };
        let applied = AppliedCameraState {
            camera_id: "rear".into(),
            width: 1280,
            height: 720,
            fps: 30.0,
            ..Default::default()
        };
        let mut session = CameraSession::new("session-1".into(), now);
        session.activate(
            caps,
            applied,
            VideoFormat {
                width: 1280,
                height: 720,
                fps: 30,
            },
            now,
        );
        session
    }

    fn command(session: &CameraSession, id: &str) -> CameraCommand {
        CameraCommand {
            session_id: session.session_id.clone(),
            command_id: id.into(),
            expected_revision: session.revision,
            controls: RequestedControls {
                torch: None,
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                ..Default::default()
            },
        }
    }

    #[test]
    fn phone_heartbeat_loss_cancels_and_closes_session() {
        let mut session = live_session();
        let lost_at = session.heartbeat + PHONE_HEARTBEAT_TIMEOUT + Duration::from_millis(1);
        assert!(session.expire_heartbeat_if_needed(lost_at));
        assert_eq!(session.status, "stopped");
        assert!(session.cancel.load(Ordering::Acquire));
        assert!(session.pending.is_none());
        assert!(!session.expire_heartbeat_if_needed(lost_at + Duration::from_secs(1)));
    }

    #[test]
    fn acknowledgement_requires_fresh_command_and_actual_applied_state() {
        let mut session = live_session();
        session
            .enqueue_command(command(&session, "cmd-1"), Instant::now())
            .unwrap();
        let wrong = PhoneHeartbeat {
            session_id: session.session_id.clone(),
            capabilities: None,
            applied: Some(session.applied.clone().unwrap()),
            command_id: Some("old-command".into()),
            error: None,
            stats: None,
        };
        assert_eq!(
            session.accept_heartbeat(wrong, Instant::now()),
            Err(HeartbeatError::StaleAcknowledgement)
        );
        assert!(session.pending.is_some());

        let missing = PhoneHeartbeat {
            session_id: session.session_id.clone(),
            capabilities: None,
            applied: None,
            command_id: Some("cmd-1".into()),
            error: None,
            stats: None,
        };
        assert_eq!(
            session.accept_heartbeat(missing, Instant::now()),
            Err(HeartbeatError::MissingAppliedState)
        );
        assert!(session.pending.is_some());

        let actual = session.applied.clone().unwrap();
        let accepted = PhoneHeartbeat {
            session_id: session.session_id.clone(),
            capabilities: None,
            applied: Some(actual),
            command_id: Some("cmd-1".into()),
            error: None,
            stats: None,
        };
        assert!(session.accept_heartbeat(accepted, Instant::now()).is_ok());
        assert!(session.pending.is_none());
        assert_eq!(session.last_command_id.as_deref(), Some("cmd-1"));
        assert_eq!(
            session.last_command_outcome.as_ref().unwrap().status,
            CommandOutcomeStatus::Applied
        );
        assert_eq!(session.revision, 2);
    }

    #[test]
    fn requested_1080p30_applied_720p30_clears_pending_and_reports_adjustment() {
        let mut session = live_session();
        let now = Instant::now();
        session.output_format = Some(VideoFormat {
            width: 1920,
            height: 1080,
            fps: 30,
        });
        session.applied = Some(AppliedCameraState {
            camera_id: "rear".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            ..Default::default()
        });
        let mut requested = command(&session, "cmd-adjusted");
        requested.controls.width = Some(1920);
        requested.controls.height = Some(1080);
        session.enqueue_command(requested, now).unwrap();

        let actual = AppliedCameraState {
            camera_id: "rear".into(),
            width: 1280,
            height: 720,
            fps: 30.0,
            ..Default::default()
        };
        session
            .accept_heartbeat(
                PhoneHeartbeat {
                    session_id: session.session_id.clone(),
                    capabilities: None,
                    applied: Some(actual.clone()),
                    command_id: Some("cmd-adjusted".into()),
                    error: None,
                    stats: None,
                },
                now,
            )
            .unwrap();

        assert!(session.pending.is_none());
        assert_eq!(session.last_command_id.as_deref(), Some("cmd-adjusted"));
        assert_eq!(session.applied.as_ref().unwrap().width, 1280);
        assert_eq!(session.applied.as_ref().unwrap().height, 720);
        let outcome = session.last_command_outcome.as_ref().unwrap();
        assert_eq!(outcome.status, CommandOutcomeStatus::Adjusted);
        assert!(outcome.message.as_deref().unwrap().contains("1280x720"));
        assert_eq!(session.error, outcome.message);
        assert_eq!(session.revision, 2);
        let snapshot = session.refresh_snapshot();
        assert_eq!(snapshot["lastCommandOutcome"]["status"], "adjusted");
        assert_eq!(snapshot["applied"]["width"], 1280);
        assert!(snapshot["pending"].is_null());
    }

    #[test]
    fn live_resolution_or_fps_change_is_rejected_with_backend_reason() {
        let mut session = live_session();
        let mut resolution = command(&session, "cmd-resolution");
        resolution.controls.width = Some(1920);
        resolution.controls.height = Some(1080);
        assert_eq!(
            session.enqueue_command(resolution, Instant::now()),
            Err(CommandError::InvalidControls(
                LIVE_FORMAT_CHANGE_UNAVAILABLE
            ))
        );

        let mut fps = command(&session, "cmd-fps");
        fps.controls.fps = Some(60.0);
        assert_eq!(
            session.enqueue_command(fps, Instant::now()),
            Err(CommandError::InvalidControls(
                LIVE_FORMAT_CHANGE_UNAVAILABLE
            ))
        );
    }

    #[test]
    fn phone_apply_error_is_an_acknowledged_rejection() {
        let mut session = live_session();
        session
            .enqueue_command(command(&session, "cmd-rejected"), Instant::now())
            .unwrap();
        session
            .accept_heartbeat(
                PhoneHeartbeat {
                    session_id: session.session_id.clone(),
                    capabilities: None,
                    applied: session.applied.clone(),
                    command_id: Some("cmd-rejected".into()),
                    error: Some("constraint rejected".into()),
                    stats: None,
                },
                Instant::now(),
            )
            .unwrap();
        assert!(session.pending.is_none());
        assert_eq!(session.last_command_id.as_deref(), Some("cmd-rejected"));
        let outcome = session.last_command_outcome.unwrap();
        assert_eq!(outcome.status, CommandOutcomeStatus::Rejected);
        assert_eq!(outcome.message.as_deref(), Some("constraint rejected"));
    }

    #[test]
    fn stop_preempts_pending_command_and_is_terminal() {
        let mut session = live_session();
        session
            .enqueue_command(command(&session, "cmd-1"), Instant::now())
            .unwrap();
        let mut stop = command(&session, "stop");
        stop.expected_revision = 0;
        stop.controls.stop = Some(true);
        session.enqueue_command(stop, Instant::now()).unwrap();
        assert_eq!(session.status, "stopped");
        assert!(session.cancel.load(Ordering::Acquire));
        assert!(session.pending.is_none());
        assert!(
            session
                .enqueue_command(command(&session, "cmd-2"), Instant::now())
                .is_err()
        );
    }

    #[test]
    fn unacknowledged_commands_expire_and_cannot_be_replayed() {
        let mut session = live_session();
        let now = Instant::now();
        session
            .enqueue_command(command(&session, "cmd-1"), now)
            .unwrap();
        assert!(session.expire_pending_if_needed(now + COMMAND_TIMEOUT));
        assert!(session.pending.is_none());
        assert_eq!(
            session.error.as_deref(),
            Some("Phone did not acknowledge the camera command before its deadline")
        );
        assert_eq!(
            session.enqueue_command(command(&session, "cmd-1"), now + COMMAND_TIMEOUT),
            Err(CommandError::ReusedId)
        );
    }

    #[test]
    fn desktop_and_phone_bearers_are_role_bound() {
        use axum::http::HeaderValue;
        let state = AppState::new(
            "phone-secret".into(),
            "desktop-secret".into(),
            "192.168.1.5:8443".parse().unwrap(),
            "/dev/video0".into(),
            "session-1".into(),
            Instant::now(),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer phone-secret"),
        );
        assert!(authorized(&headers, &state, Role::Phone));
        assert!(!authorized(&headers, &state, Role::Desktop));
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer desktop-secret"),
        );
        assert!(authorized(&headers, &state, Role::Desktop));
        assert!(!authorized(&headers, &state, Role::Phone));
    }
}
