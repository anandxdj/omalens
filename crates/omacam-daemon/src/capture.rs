//! Explicit-consent capture session and authenticated media ingress.
//!
//! The control server mints a binding only after phone approval, then hands
//! ownership to this module. All exit paths invalidate policy first, write a
//! terminal Stop to the isolated output worker, and tear down owned resources.

use std::env;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use omacam_core::SessionPolicy;
use omacam_core::control::{ControlProof, ControlSession};
use omacam_core::media::{
    ControlConnectionId, MEDIA_HEADER_BYTES, MediaBinding, MediaRecord, MediaRecordKind,
    MediaSessionId, MediaStreamValidator, PeerIdentity, write_record,
};
use tokio::io::{AsyncReadExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, timeout};
use tokio_rustls::TlsAcceptor;

use crate::CLIENT_STEP_TIMEOUT;
use crate::control_server::{
    ControlCommand, ControlConnectionOutcome, ControlReply, read_json_line, trust_record_matches,
    write_json_line,
};

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
    output_device: &Path,
) -> Result<ControlConnectionOutcome, Box<dyn std::error::Error>> {
    let mut output = OutputWorker::spawn(output_device, binding)?;
    let (media_tx, mut media_rx) = mpsc::channel::<Result<MediaRecord, String>>(2);
    let media_acceptor = acceptor.clone();
    let certificate = certificate_der.to_vec();
    let phone_key = trusted_public_key.to_vec();
    tokio::spawn(async move {
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
            let _ = media_tx.send(Err(error)).await;
        }
    });

    let mut lease_tick = interval(Duration::from_millis(250));
    let mut last_control = Instant::now();
    let mut output_sequence = 0_u64;
    let mut capture_started = false;
    let mut outcome = ControlConnectionOutcome::Disconnected;
    let stop_reason: String;

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    policy.connection_lost();
                    stop_reason = "authenticated control connection ended".to_owned();
                    break;
                };
                let command = command?;
                if !trust_record_matches(trust_path, trusted_public_key) {
                    policy.forget_peer();
                    stop_reason = "trusted phone was revoked while capture was active".to_owned();
                    break;
                }
                match command {
                    ControlCommand::Ping => {
                        last_control = Instant::now();
                        policy.refresh_lease(binding.generation, monotonic_millis())?;
                        write_json_line(
                            control_write,
                            &ControlReply::Pong { capture_authorized: true },
                        ).await?;
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
                        break;
                    }
                    ControlCommand::ForgetPeer => {
                        policy.forget_peer();
                        crate::forget_trust(trust_path)
                            .map_err(|error| format!("revocation failed: {error}"))?;
                        outcome = ControlConnectionOutcome::ForgotPeer;
                        stop_reason = "phone revoked desktop trust".to_owned();
                        break;
                    }
                    ControlCommand::StartApproved { .. }
                    | ControlCommand::StartRejected { .. }
                    | ControlCommand::MediaOpen { .. } => {
                        policy.stop();
                        stop_reason = "invalid command for active capture".to_owned();
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
                            policy.capture_started(binding.generation)?;
                            capture_started = true;
                        }
                        policy.accept_frame(binding.generation, monotonic_millis())?;
                        record.sequence = output_sequence;
                        output_sequence = output_sequence.checked_add(1)
                            .ok_or("output media sequence exhausted")?;
                        output.write(binding, &record)?;
                    }
                    Some(Err(error)) => {
                        policy.stop();
                        stop_reason = format!("authenticated media failed: {error}");
                        break;
                    }
                    None => {
                        policy.stop();
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
            signal = tokio::signal::ctrl_c() => {
                signal?;
                policy.stop();
                let _ = write_json_line(
                    control_write,
                    &ControlReply::StopCapture { reason: "desktop requested Stop" },
                ).await;
                stop_reason = "desktop requested Stop".to_owned();
                break;
            }
        }
    }

    output.write_stop(binding, output_sequence);
    let _ = write_json_line(
        control_write,
        &ControlReply::Stopped {
            reason: stop_reason.clone(),
        },
    )
    .await;
    println!("Capture stopped: {stop_reason}.");

    // Keep the healthy writer alive on its permanent neutral branch until the
    // authenticated control session itself ends. This preserves the consumer
    // handle and makes Stop distinct from shutting down the output service.
    loop {
        let command = tokio::select! {
            command = command_rx.recv() => command,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(ControlConnectionOutcome::Shutdown);
            }
        };
        let Some(command) = command else {
            return Ok(outcome);
        };
        match command? {
            ControlCommand::Ping => {
                write_json_line(
                    control_write,
                    &ControlReply::Pong {
                        capture_authorized: false,
                    },
                )
                .await?;
            }
            ControlCommand::Disconnect => return Ok(outcome),
            ControlCommand::ForgetPeer => {
                policy.forget_peer();
                crate::forget_trust(trust_path)
                    .map_err(|error| format!("revocation failed: {error}"))?;
                let _ = write_json_line(control_write, &ControlReply::Forgotten).await;
                return Ok(ControlConnectionOutcome::ForgotPeer);
            }
            ControlCommand::Stop => {}
            ControlCommand::StartApproved { .. }
            | ControlCommand::StartRejected { .. }
            | ControlCommand::MediaOpen { .. } => {
                return Err("capture command arrived after terminal Stop".into());
            }
        }
    }
}

async fn receive_authenticated_media(
    listener: Arc<TcpListener>,
    acceptor: TlsAcceptor,
    certificate_der: Vec<u8>,
    trusted_public_key: Vec<u8>,
    binding: MediaBinding,
    sender: mpsc::Sender<Result<MediaRecord, String>>,
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
            sender
                .send(Ok(record))
                .await
                .map_err(|_| "media consumer stopped".to_owned())?;
            return Ok(());
        }
        match sender.try_send(Ok(record)) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
        }
    }
}

struct OutputWorker {
    child: Child,
    input: Option<ChildStdin>,
    binding: MediaBinding,
    next_sequence: u64,
    terminal_sent: bool,
}

impl OutputWorker {
    fn spawn(device: &Path, binding: MediaBinding) -> Result<Self, Box<dyn std::error::Error>> {
        let executable = env::current_exe()?
            .parent()
            .map(|parent| parent.join("omacam-output"))
            .filter(|path| path.exists())
            .unwrap_or_else(|| "omacam-output".into());
        let mut child = Command::new(executable)
            .arg("--device")
            .arg(device)
            .args([
                "--framed-h264-stdin".to_owned(),
                "--peer".to_owned(),
                binding.peer_identity.to_hex(),
                "--connection".to_owned(),
                binding.control_connection_id.to_hex(),
                "--session".to_owned(),
                binding.media_session_id.to_hex(),
                "--generation".to_owned(),
                binding.generation.to_string(),
            ])
            .stdin(Stdio::piped())
            .spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or("output worker did not expose media input")?;
        Ok(Self {
            child,
            input: Some(input),
            binding,
            next_sequence: 0,
            terminal_sent: false,
        })
    }

    fn write(
        &mut self,
        binding: MediaBinding,
        record: &MediaRecord,
    ) -> Result<(), Box<dyn std::error::Error>> {
        write_record(
            self.input
                .as_mut()
                .ok_or("output worker media input is closed")?,
            binding,
            record,
        )?;
        self.next_sequence = record.sequence.saturating_add(1);
        Ok(())
    }

    fn write_stop(&mut self, binding: MediaBinding, sequence: u64) {
        if self.terminal_sent {
            return;
        }
        let stop = MediaRecord {
            kind: MediaRecordKind::Stop,
            key_frame: false,
            sequence,
            presentation_time_us: 0,
            payload: Vec::new(),
        };
        if self.write(binding, &stop).is_ok() {
            self.terminal_sent = true;
        }
        self.input.take();
    }

    fn stop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for OutputWorker {
    fn drop(&mut self) {
        let binding = self.binding;
        let sequence = self.next_sequence;
        self.write_stop(binding, sequence);
        self.stop();
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

    #[test]
    fn start_response_must_match_the_exact_pending_request() {
        let expected = [7_u8; 16];
        assert!(require_request_id(Some(expected), &URL_SAFE_NO_PAD.encode(expected)).is_ok());
        assert!(require_request_id(Some(expected), &URL_SAFE_NO_PAD.encode([8_u8; 16])).is_err());
        assert!(require_request_id(None, &URL_SAFE_NO_PAD.encode(expected)).is_err());
        assert!(require_request_id(Some(expected), "malformed").is_err());
    }
}
