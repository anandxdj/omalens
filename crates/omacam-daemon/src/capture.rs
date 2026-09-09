//! Explicit-consent capture session and authenticated media ingress.
//!
//! The control server mints a binding only after phone approval, then hands
//! ownership to this module. All exit paths invalidate policy first, write a
//! terminal Stop to the isolated output worker, and tear down owned resources.

use std::env;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use omacam_core::SessionPolicy;
use omacam_core::control::{ControlProof, ControlSession};
use omacam_core::media::{
    ControlConnectionId, MEDIA_HEADER_BYTES, MediaBinding, MediaRecord, MediaRecordKind,
    MediaSessionId, MediaStreamValidator, OutputControlCommand, PeerIdentity, write_output_control,
    write_record,
};
use tokio::io::{AsyncReadExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, interval, sleep_until, timeout};
use tokio_rustls::TlsAcceptor;

use crate::CLIENT_STEP_TIMEOUT;
use crate::control_server::{
    ControlCommand, ControlConnectionOutcome, ControlReply, read_json_line, trust_record_matches,
    write_json_line,
};
use crate::service_runtime::{ServiceIntent, ServiceRuntime};

const OUTPUT_NEUTRALIZATION_MS: u64 = 500;
const MEDIA_QUEUE_CAPACITY: usize = 2;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) async fn run_active_capture<W: tokio::io::AsyncWrite + Unpin>(
    listener: Arc<TcpListener>,
    acceptor: &TlsAcceptor,
    certificate_der: &[u8],
    trusted_public_key: &[u8],
    trust_path: &Path,
    control_write: &mut W,
    command_rx: &mut mpsc::Receiver<Result<ControlCommand, String>>,
    policy: &mut SessionPolicy,
    binding: MediaBinding,
    output: Option<&mut OutputWorker>,
    mut runtime: Option<(&ServiceRuntime, &mut mpsc::Receiver<ServiceIntent>)>,
    start_operation_id: Option<String>,
) -> Result<ControlConnectionOutcome, Box<dyn std::error::Error>> {
    let Some(output) = output else {
        policy.stop();
        if let Some((state, _)) = &runtime {
            state.publish_policy(policy);
            if let Some(operation_id) = &start_operation_id {
                state.fail(
                    operation_id,
                    "output_unavailable",
                    "output worker is unavailable",
                );
            }
        }
        return Err("output worker is unavailable".into());
    };
    if let Err(error) = output.bind(binding).await {
        policy.stop();
        if let Some((state, _)) = &runtime {
            state.set_output_failed(error.to_string());
            state.publish_policy(policy);
            if let Some(operation_id) = &start_operation_id {
                state.fail(operation_id, "output_start_failed", error.to_string());
            }
        }
        return Err(error);
    }

    let (media_tx, mut media_rx) = fresh_media_channel();
    let media_acceptor = acceptor.clone();
    let certificate = certificate_der.to_vec();
    let phone_key = trusted_public_key.to_vec();
    let media_task = tokio::spawn(async move {
        let result = receive_authenticated_media(
            listener,
            media_acceptor,
            certificate,
            phone_key,
            binding,
            media_tx.clone(),
        )
        .await;
        if let Err(error) = result {
            media_tx.send(Err(error));
        }
    });

    let mut lease_tick = interval(Duration::from_millis(250));
    let mut output_health_tick = interval(Duration::from_millis(250));
    let mut last_control = Instant::now();
    let mut output_sequence = 0_u64;
    let mut capture_started = false;
    let mut outcome = ControlConnectionOutcome::CaptureStopped;
    let mut stop_reason: String;
    let mut fatal_error: Option<Box<dyn std::error::Error>> = None;

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    policy.connection_lost();
                    stop_reason = "authenticated control connection ended".to_owned();
                    outcome = ControlConnectionOutcome::Disconnected;
                    break;
                };
                let command = match command {
                    Ok(command) => command,
                    Err(error) => {
                        policy.stop();
                        stop_reason = error;
                        fatal_error = Some("authenticated control reader failed".into());
                        break;
                    }
                };
                if !trust_record_matches(trust_path, trusted_public_key) {
                    policy.forget_peer();
                    stop_reason = "trusted phone was revoked while capture was active".to_owned();
                    outcome = ControlConnectionOutcome::ForgotPeer;
                    break;
                }
                match command {
                    ControlCommand::Ping => {
                        last_control = Instant::now();
                        if let Err(error) = policy.refresh_lease(binding.generation, monotonic_millis()) {
                            fatal_error = Some(error.into());
                            policy.stop();
                            stop_reason = "capture authorization lease could not be refreshed".to_owned();
                            break;
                        }
                        if let Err(error) = write_json_line(
                            control_write,
                            &ControlReply::Pong { capture_authorized: true },
                        ).await {
                            fatal_error = Some(error);
                            policy.connection_lost();
                            stop_reason = "authenticated control connection ended".to_owned();
                            outcome = ControlConnectionOutcome::Disconnected;
                            break;
                        }
                    }
                    ControlCommand::Stop => {
                        policy.stop();
                        stop_reason = "phone stopped capture".to_owned();
                        break;
                    }
                    ControlCommand::Disconnect => {
                        policy.connection_lost();
                        policy.stop();
                        stop_reason = "authenticated control disconnected".to_owned();
                        outcome = ControlConnectionOutcome::Disconnected;
                        break;
                    }
                    ControlCommand::ForgetPeer => {
                        policy.forget_peer();
                        if let Some((state, _)) = &runtime { state.publish_policy(policy); }
                        match crate::forget_trust(trust_path) {
                            Ok(()) => {
                                outcome = ControlConnectionOutcome::ForgotPeer;
                                stop_reason = "phone revoked desktop trust".to_owned();
                            }
                            Err(error) => {
                                fatal_error = Some(format!("revocation failed: {error}").into());
                                outcome = ControlConnectionOutcome::Disconnected;
                                stop_reason = "phone revoked desktop trust".to_owned();
                            }
                        }
                        break;
                    }
                    ControlCommand::StartApproved { .. }
                    | ControlCommand::StartRejected { .. }
                    | ControlCommand::MediaOpen { .. } => {
                        policy.stop();
                        stop_reason = "invalid command for active capture".to_owned();
                        fatal_error = Some("invalid command for active capture".into());
                        break;
                    }
                }
            }
            media = media_rx.recv() => {
                match media {
                    Some(Ok(mut record)) => {
                        if record.kind == MediaRecordKind::Stop {
                            policy.stop();
                            stop_reason = "phone media channel sent terminal Stop".to_owned();
                            break;
                        }
                        if !capture_started {
                            if let Err(error) = policy.capture_started(binding.generation) {
                                fatal_error = Some(error.into());
                                policy.stop();
                                stop_reason = "capture generation could not start".to_owned();
                                break;
                            }
                            capture_started = true;
                            if let Some((state, _)) = &runtime {
                                state.publish_policy(policy);
                                if let Some(operation_id) = &start_operation_id {
                                    state.succeed(operation_id);
                                }
                            }
                        }
                        if let Err(error) = policy.accept_frame(binding.generation, monotonic_millis()) {
                            fatal_error = Some(error.into());
                            policy.stop();
                            stop_reason = "capture frame generation was rejected".to_owned();
                            break;
                        }
                        if let Some((state, _)) = &runtime { state.publish_policy(policy); }
                        record.sequence = output_sequence;
                        let Some(next_sequence) = output_sequence.checked_add(1) else {
                            fatal_error = Some("output media sequence exhausted".into());
                            policy.stop();
                            stop_reason = "output media sequence exhausted".to_owned();
                            break;
                        };
                        output_sequence = next_sequence;
                        if let Err(error) = output.write(binding, &record) {
                            if let Some((state, _)) = &runtime {
                                state.set_output_failed(error.to_string());
                            }
                            policy.stop();
                            stop_reason = format!("output worker failed: {error}");
                            break;
                        }
                    }
                    Some(Err(error)) => {
                        policy.stop();
                        if let Some((state, _)) = &runtime { state.publish_policy(policy); }
                        stop_reason = format!("authenticated media failed: {error}");
                        break;
                    }
                    None => {
                        policy.stop();
                        if let Some((state, _)) = &runtime { state.publish_policy(policy); }
                        stop_reason = "authenticated media receiver ended".to_owned();
                        break;
                    }
                }
            }
            _ = lease_tick.tick() => {
                policy.tick(monotonic_millis());
                if last_control.elapsed() >= Duration::from_secs(10)
                    || !policy.snapshot().capture_armed
                {
                    policy.stop();
                    stop_reason = "capture authorization lease expired".to_owned();
                    break;
                }
            }
            _ = output_health_tick.tick() => {
                if let Err(error) = output.check_health() {
                    if let Some((state, _)) = &runtime {
                        state.set_output_failed(error.clone());
                    }
                    policy.stop();
                    stop_reason = format!("output worker failed: {error}");
                    break;
                }
            }
            intent = async {
                match runtime.as_mut() {
                    Some((_, receiver)) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let intent = intent.ok_or("service intent channel stopped")?;
                match intent {
                    ServiceIntent::Stop { operation_id } => {
                        policy.stop();
                        if let Some((state, _)) = &runtime {
                            state.publish_policy(policy);
                            state.succeed(&operation_id);
                        }
                        let _ = write_json_line(control_write, &ControlReply::StopCapture { reason: "desktop requested Stop" }).await;
                        stop_reason = "desktop requested Stop".to_owned();
                        break;
                    }
                    ServiceIntent::ForgetPeer { operation_id } => {
                        policy.forget_peer();
                        if let Some((state, _)) = &runtime { state.publish_policy(policy); }
                        match crate::forget_trust(trust_path) {
                            Ok(()) => if let Some((state, _)) = &runtime { state.succeed(&operation_id); },
                            Err(error) => if let Some((state, _)) = &runtime { state.fail(&operation_id, "forget_failed", error); },
                        }
                        outcome = ControlConnectionOutcome::ForgotPeer;
                        stop_reason = "desktop forgot trusted phone".to_owned();
                        break;
                    }
                    ServiceIntent::RequestStart { operation_id } => {
                        if let Some((state, _)) = &runtime { state.fail(&operation_id, "capture_busy", "capture is already active"); }
                    }
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal?;
                policy.stop();
                let _ = write_json_line(
                    control_write,
                    &ControlReply::StopCapture { reason: "desktop requested Stop" },
                ).await;
                stop_reason = "desktop requested Stop".to_owned();
                outcome = ControlConnectionOutcome::Shutdown;
                break;
            }
        }
    }

    media_task.abort();
    let _ = media_task.await;
    if let Err(error) = output.reset(binding) {
        if let Some((state, _)) = &runtime {
            state.set_output_failed(error.to_string());
        }
        if fatal_error.is_none() {
            stop_reason = format!("output reset failed: {error}");
        }
    }
    if let (Some(operation_id), Some((state, _))) = (&start_operation_id, &runtime) {
        state.fail(operation_id, "capture_stopped", stop_reason.clone());
    }
    if let Some((state, _)) = &runtime {
        state.publish_policy(policy);
    }
    let _ = write_json_line(
        control_write,
        &ControlReply::Stopped {
            reason: stop_reason.clone(),
        },
    )
    .await;
    println!("Capture stopped: {stop_reason}.");

    if let Some(error) = fatal_error {
        // A connection/parser failure still tears down the authenticated
        // session. A clean Stop returns to the control loop so later D-Bus
        // intents can be processed on the same connection.
        if matches!(outcome, ControlConnectionOutcome::CaptureStopped) {
            return Err(error);
        }
    }
    Ok(outcome)
}

#[derive(Clone)]
struct FreshMediaSender {
    state: Arc<Mutex<FreshMediaState>>,
    notify: Arc<Notify>,
}

#[derive(Clone)]
struct FreshMediaReceiver {
    state: Arc<Mutex<FreshMediaState>>,
    notify: Arc<Notify>,
}

struct FreshMediaState {
    queue: std::collections::VecDeque<Result<MediaRecord, String>>,
    closed: bool,
}

fn fresh_media_channel() -> (FreshMediaSender, FreshMediaReceiver) {
    let state = Arc::new(Mutex::new(FreshMediaState {
        queue: std::collections::VecDeque::with_capacity(MEDIA_QUEUE_CAPACITY),
        closed: false,
    }));
    let notify = Arc::new(Notify::new());
    (
        FreshMediaSender {
            state: Arc::clone(&state),
            notify: Arc::clone(&notify),
        },
        FreshMediaReceiver { state, notify },
    )
}

impl FreshMediaSender {
    fn send(&self, item: Result<MediaRecord, String>) {
        let terminal = item.is_err()
            || item
                .as_ref()
                .is_ok_and(|record| record.kind == MediaRecordKind::Stop);
        let mut state = self.state.lock().expect("media queue mutex poisoned");
        if state.closed {
            return;
        }
        if terminal {
            // Terminal/error notifications outrank every queued access unit.
            state.queue.clear();
            state.queue.push_back(item);
            state.closed = true;
        } else {
            while state.queue.len() >= MEDIA_QUEUE_CAPACITY {
                state.queue.pop_front();
            }
            state.queue.push_back(item);
        }
        drop(state);
        self.notify.notify_one();
    }
}

impl FreshMediaReceiver {
    async fn recv(&mut self) -> Option<Result<MediaRecord, String>> {
        loop {
            let notified = self.notify.notified();
            let should_wait = {
                let mut state = self.state.lock().expect("media queue mutex poisoned");
                if let Some(item) = state.queue.pop_front() {
                    return Some(item);
                }
                !state.closed
            };
            if !should_wait {
                return None;
            }
            notified.await;
        }
    }
}

async fn receive_authenticated_media(
    listener: Arc<TcpListener>,
    acceptor: TlsAcceptor,
    certificate_der: Vec<u8>,
    trusted_public_key: Vec<u8>,
    binding: MediaBinding,
    sender: FreshMediaSender,
) -> Result<(), String> {
    let (socket, _) = timeout(Duration::from_secs(10), listener.accept())
        .await
        .map_err(|_| "media connection timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    let tls_stream = timeout(CLIENT_STEP_TIMEOUT, acceptor.accept(socket))
        .await
        .map_err(|_| "media TLS handshake timed out".to_owned())?
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut session =
        ControlSession::create(&certificate_der, 0).map_err(|error| error.to_string())?;
    let mut stream = BufReader::new(tls_stream);
    write_json_line(stream.get_mut(), session.hello())
        .await
        .map_err(|error| error.to_string())?;
    let proof: ControlProof = read_json_line(&mut stream)
        .await
        .map_err(|error| error.to_string())?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    session
        .authenticate(&proof, &trusted_public_key, elapsed_ms)
        .map_err(|error| error.to_string())?;
    write_json_line(
        stream.get_mut(),
        &ControlReply::Authenticated {
            version: 1,
            capture_authorized: false,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let open: ControlCommand = read_json_line(&mut stream)
        .await
        .map_err(|error| error.to_string())?;
    let ControlCommand::MediaOpen {
        peer,
        connection,
        media_session,
        generation,
    } = open
    else {
        return Err("authenticated media connection did not send media_open".to_owned());
    };
    if PeerIdentity::from_hex(&peer).map_err(|error| error.to_string())? != binding.peer_identity
        || ControlConnectionId::from_hex(&connection).map_err(|error| error.to_string())?
            != binding.control_connection_id
        || MediaSessionId::from_hex(&media_session).map_err(|error| error.to_string())?
            != binding.media_session_id
        || generation != binding.generation
    {
        return Err("media_open binding does not match approved capture".to_owned());
    }
    write_json_line(stream.get_mut(), &ControlReply::MediaReady)
        .await
        .map_err(|error| error.to_string())?;

    let mut validator = MediaStreamValidator::new(binding);
    loop {
        let mut header = [0_u8; MEDIA_HEADER_BYTES];
        timeout(CLIENT_STEP_TIMEOUT, stream.read_exact(&mut header))
            .await
            .map_err(|_| "media header timed out".to_owned())?
            .map_err(|error| error.to_string())?;
        let pending = validator
            .validate_header(&header)
            .map_err(|error| error.to_string())?;
        let mut payload = vec![0_u8; pending.payload_length()];
        timeout(CLIENT_STEP_TIMEOUT, stream.read_exact(&mut payload))
            .await
            .map_err(|_| "media payload timed out".to_owned())?
            .map_err(|error| error.to_string())?;
        let record = validator
            .finish_record(pending, payload)
            .map_err(|error| error.to_string())?;
        let terminal = record.kind == MediaRecordKind::Stop;
        if terminal {
            sender.send(Ok(record));
            return Ok(());
        }
        sender.send(Ok(record));
    }
}

pub(crate) struct OutputWorker {
    child: Child,
    input: Option<ChildStdin>,
    current_binding: Option<MediaBinding>,
    neutral_until: Option<Instant>,
    failure: Option<String>,
}

impl OutputWorker {
    pub(crate) fn spawn_service(
        device: &Path,
        preview_socket: Option<&Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let executable = env::current_exe()?
            .parent()
            .map(|parent| parent.join("omacam-output"))
            .filter(|path| path.exists())
            .unwrap_or_else(|| "omacam-output".into());
        // Keep the worker itself a child of the daemon and arrange for it to
        // receive TERM if the daemon disappears. Its GStreamer child has its
        // own parent-death signal in omacam-output.
        let mut command = Command::new("setpriv");
        command.args(["--pdeathsig", "TERM"]).arg(executable);
        command.arg("--device").arg(device).arg("--service-stdin");
        if let Some(socket) = preview_socket {
            command.arg("--preview-socket").arg(socket);
        }
        let mut child = command.stdin(Stdio::piped()).spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or("output worker did not expose media input")?;
        let mut worker = Self {
            child,
            input: Some(input),
            current_binding: None,
            neutral_until: None,
            failure: None,
        };
        worker
            .check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Ok(worker)
    }

    pub(crate) fn check_health(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        match self.child.try_wait() {
            Ok(None) => Ok(()),
            Ok(Some(status)) => {
                let error = format!("output worker exited unexpectedly with {status}");
                self.failure = Some(error.clone());
                self.input.take();
                Err(error)
            }
            Err(error) => {
                let message = format!("could not inspect output worker: {error}");
                self.failure = Some(message.clone());
                self.input.take();
                Err(message)
            }
        }
    }

    pub(crate) fn is_healthy(&mut self) -> bool {
        self.check_health().is_ok()
    }

    pub(crate) async fn bind(
        &mut self,
        binding: MediaBinding,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.current_binding.is_some() {
            return Err("output binding requires a reset before rebind".into());
        }
        if let Some(deadline) = self.neutral_until {
            sleep_until(deadline).await;
        }
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let input = self
            .input
            .as_mut()
            .ok_or("output worker media input is closed")?;
        write_output_control(input, OutputControlCommand::Bind(binding))?;
        std::io::Write::flush(input)?;
        self.current_binding = Some(binding);
        self.neutral_until = None;
        Ok(())
    }

    pub(crate) fn reset(
        &mut self,
        binding: MediaBinding,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self
            .current_binding
            .is_some_and(|current| current != binding)
        {
            return Err("output reset belongs to a different generation".into());
        }
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if self.current_binding.is_some() {
            let input = self
                .input
                .as_mut()
                .ok_or("output worker media input is closed")?;
            write_output_control(input, OutputControlCommand::Reset(binding))?;
            std::io::Write::flush(input)?;
        }
        self.current_binding = None;
        self.neutral_until = Some(Instant::now() + Duration::from_millis(OUTPUT_NEUTRALIZATION_MS));
        Ok(())
    }

    fn write(
        &mut self,
        binding: MediaBinding,
        record: &MediaRecord,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.check_health()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if self.current_binding != Some(binding) {
            return Err("output frame belongs to a stale or unbound generation".into());
        }
        if record.kind != MediaRecordKind::H264AccessUnit {
            return Err("output worker accepts access units only; use Reset for Stop".into());
        }
        write_record(
            self.input
                .as_mut()
                .ok_or("output worker media input is closed")?,
            binding,
            record,
        )?;
        std::io::Write::flush(
            self.input
                .as_mut()
                .ok_or("output worker media input is closed")?,
        )?;
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        if self.input.is_some()
            && self.failure.is_none()
            && let Some(input) = self.input.as_mut()
        {
            let _ = write_output_control(input, OutputControlCommand::Shutdown);
            let _ = std::io::Write::flush(input);
        }
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for OutputWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn require_request_id(
    expected: Option<[u8; 16]>,
    received: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let expected = expected.ok_or("no desktop Start request is pending")?;
    let decoded = URL_SAFE_NO_PAD
        .decode(received)
        .map_err(|_| "Start request identifier is malformed")?;
    if decoded != expected {
        return Err("Start response does not match the pending request".into());
    }
    Ok(())
}

pub(crate) fn random_identifier<const N: usize>() -> Result<[u8; N], Box<dyn std::error::Error>> {
    let mut value = [0_u8; N];
    getrandom::fill(&mut value).map_err(|_| "secure random source unavailable")?;
    Ok(value)
}

pub(crate) fn monotonic_millis() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn access_unit(sequence: u64) -> MediaRecord {
        MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: sequence == 0,
            sequence,
            presentation_time_us: sequence + 1,
            payload: vec![0, 0, 0, 1, 0x65],
        }
    }

    #[test]
    fn start_response_must_match_the_exact_pending_request() {
        let expected = [7_u8; 16];
        assert!(require_request_id(Some(expected), &URL_SAFE_NO_PAD.encode(expected)).is_ok());
        assert!(require_request_id(Some(expected), &URL_SAFE_NO_PAD.encode([8_u8; 16])).is_err());
        assert!(require_request_id(None, &URL_SAFE_NO_PAD.encode(expected)).is_err());
        assert!(require_request_id(Some(expected), "malformed").is_err());
    }

    #[tokio::test]
    async fn media_queue_keeps_the_freshest_two_frames() {
        let (sender, mut receiver) = fresh_media_channel();
        sender.send(Ok(access_unit(0)));
        sender.send(Ok(access_unit(1)));
        sender.send(Ok(access_unit(2)));

        assert_eq!(receiver.recv().await.unwrap().unwrap().sequence, 1);
        assert_eq!(receiver.recv().await.unwrap().unwrap().sequence, 2);
    }

    #[tokio::test]
    async fn media_queue_gives_terminal_stop_priority_over_queued_frames() {
        let (sender, mut receiver) = fresh_media_channel();
        sender.send(Ok(access_unit(0)));
        sender.send(Ok(access_unit(1)));
        sender.send(Ok(MediaRecord {
            kind: MediaRecordKind::Stop,
            key_frame: false,
            sequence: 2,
            presentation_time_us: 0,
            payload: Vec::new(),
        }));

        assert_eq!(
            receiver.recv().await.unwrap().unwrap().kind,
            MediaRecordKind::Stop
        );
        assert!(receiver.recv().await.is_none());
    }
}
