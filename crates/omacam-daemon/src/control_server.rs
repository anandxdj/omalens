//! Authenticated control listener and explicit Start/consent protocol.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use omacam_core::SessionPolicy;
use omacam_core::control::{ControlProof, ControlSession};
use omacam_core::media::{ControlConnectionId, MediaBinding, MediaSessionId, PeerIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::PrivateKeyDer;

use crate::service_runtime::{ServiceIntent, ServiceRuntime};
use crate::{
    CLIENT_STEP_TIMEOUT, MAX_CONTROL_MESSAGE_BYTES, capture, default_data_path, forget_trust,
    hostname_display_name, load_or_create_identity, load_trust_record,
};

#[derive(Debug)]
pub(crate) struct ControlServeOptions {
    pub(crate) listen: SocketAddr,
    pub(crate) trust_path: std::path::PathBuf,
    pub(crate) identity_path: std::path::PathBuf,
    request_start: bool,
    pub(crate) output_device: Option<std::path::PathBuf>,
    pub(crate) preview_socket: Option<std::path::PathBuf>,
}

impl ControlServeOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut listen = None;
        let mut trust_path = default_data_path("trusted-phone.json");
        let mut identity_path = default_data_path("desktop-identity.json");
        let mut request_start = false;
        let mut output_device = None;
        let mut preview_socket = None;
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            if flag == "--request-start" {
                request_start = true;
                index += 1;
                continue;
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag {
                "--listen" => {
                    listen =
                        Some(value.parse::<SocketAddr>().map_err(|_| {
                            "--listen must be IP:PORT (use [IPv6]:PORT)".to_owned()
                        })?);
                }
                "--trust" => trust_path = value.into(),
                "--identity" => identity_path = value.into(),
                "--output-device" => output_device = Some(value.into()),
                "--preview-socket" => preview_socket = Some(value.into()),
                _ => return Err(format!("unknown control option: {flag}")),
            }
            index += 2;
        }
        let listen = listen.ok_or_else(|| "--listen is required".to_owned())?;
        if listen.ip().is_unspecified() || listen.ip().is_loopback() || listen.port() == 0 {
            return Err("--listen must be a concrete non-loopback address and port".to_owned());
        }
        if request_start != output_device.is_some() {
            return Err("--request-start and --output-device must be supplied together".to_owned());
        }
        if preview_socket.is_some() && !request_start {
            return Err("--preview-socket requires --request-start and --output-device".to_owned());
        }
        Ok(Self {
            listen,
            trust_path,
            identity_path,
            request_start,
            output_device,
            preview_socket,
        })
    }

    pub(crate) fn parse_owned(args: &[String]) -> Result<Self, String> {
        let mut parse_args = args.to_vec();
        if !parse_args.iter().any(|value| value == "--request-start") {
            parse_args.push("--request-start".to_owned());
        }
        let mut options = Self::parse(&parse_args)?;
        options.request_start = false;
        if options.output_device.is_none() {
            return Err("--output-device is required for the owned service".to_owned());
        }
        Ok(options)
    }
}

pub(crate) async fn run_owned_control_server(
    options: &ControlServeOptions,
    runtime: ServiceRuntime,
    mut intent_rx: mpsc::Receiver<ServiceIntent>,
) -> Result<(), Box<dyn std::error::Error>> {
    // The output worker is service-scoped: it is created before accepting any
    // authenticated control connection and survives every capture generation.
    let mut output = if let Some(device) = options.output_device.as_deref() {
        match capture::OutputWorker::spawn_service(device, options.preview_socket.as_deref()) {
            Ok(worker) => {
                runtime.set_output_ready();
                Some(worker)
            }
            Err(error) => {
                runtime.set_output_failed(error.to_string());
                eprintln!("OmaCam output unavailable: {error}");
                None
            }
        }
    } else {
        runtime.set_output_failed("owned service requires an output device");
        None
    };

    if load_trust_record(&options.trust_path).is_err() {
        let mut output_health_tick = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    break;
                }
                intent = intent_rx.recv() => {
                    let intent = intent.ok_or("service intent channel stopped")?;
                    match intent {
                        ServiceIntent::RequestStart { operation_id } => runtime.fail(
                            &operation_id,
                            "peer_not_trusted",
                            "no trusted phone is available",
                        ),
                        ServiceIntent::Stop { operation_id }
                        | ServiceIntent::ForgetPeer { operation_id } => {
                            runtime.succeed(&operation_id);
                        }
                    }
                }
                _ = output_health_tick.tick() => {
                    if let Some(worker) = output.as_mut()
                        && let Err(error) = worker.check_health()
                    {
                        runtime.set_output_failed(error);
                    }
                }
            }
        }
        if let Some(worker) = output.as_mut() {
            worker.shutdown();
        }
        return Ok(());
    }

    run_control_server_inner(options, Some((&runtime, &mut intent_rx)), output).await
}

pub(crate) async fn run_control_server(
    options: &ControlServeOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = match options.output_device.as_deref() {
        Some(device) => Some(capture::OutputWorker::spawn_service(
            device,
            options.preview_socket.as_deref(),
        )?),
        None => None,
    };
    run_control_server_inner(options, None, output).await
}

async fn run_control_server_inner(
    options: &ControlServeOptions,
    runtime: Option<(&ServiceRuntime, &mut mpsc::Receiver<ServiceIntent>)>,
    mut output: Option<capture::OutputWorker>,
) -> Result<(), Box<dyn std::error::Error>> {
    let trust = load_trust_record(&options.trust_path)?;
    let trusted_public_key = URL_SAFE_NO_PAD
        .decode(&trust.phone_public_key_der_base64url)
        .map_err(|_| "trusted phone public key is malformed")?;
    let stored_digest = URL_SAFE_NO_PAD
        .decode(&trust.phone_key_sha256_base64url)
        .map_err(|_| "trusted phone identity digest is malformed")?;
    let actual_digest = Sha256::digest(&trusted_public_key);
    if stored_digest.len() != 32 || actual_digest.as_slice() != stored_digest {
        return Err("trusted phone identity record is inconsistent".into());
    }

    let identity = load_or_create_identity(&options.identity_path)?;
    let certificate_der =
        tokio_rustls::rustls::pki_types::CertificateDer::from(identity.certificate_der.clone());
    let private_key = PrivateKeyDer::try_from(identity.private_key_der)?;
    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate_der.clone()], private_key)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let listener = Arc::new(TcpListener::bind(options.listen).await?);
    let certificate_digest = URL_SAFE_NO_PAD.encode(Sha256::digest(certificate_der.as_ref()));
    let (discovery, service_fullname) =
        advertise_control_service(options.listen, &certificate_digest)?;

    println!("OmaCam authenticated control");
    println!("Listening on {} for {}.", options.listen, trust.phone_name);
    println!("Authentication cannot start camera capture.");

    let result = run_control_accept_loop(
        listener,
        &acceptor,
        certificate_der.as_ref(),
        &trusted_public_key,
        &trust.phone_name,
        &options.trust_path,
        options.request_start,
        runtime,
        output.as_mut(),
    )
    .await;
    let _ = discovery.unregister(&service_fullname);
    let _ = discovery.shutdown();
    if let Some(worker) = output.as_mut() {
        worker.shutdown();
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlConnectionOutcome {
    Disconnected,
    ForgotPeer,
    Shutdown,
    /// Capture ended, but the authenticated control connection remains usable
    /// for pings and a later explicit Start request.
    CaptureStopped,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_control_accept_loop(
    listener: Arc<TcpListener>,
    acceptor: &TlsAcceptor,
    certificate_der: &[u8],
    trusted_public_key: &[u8],
    phone_name: &str,
    trust_path: &Path,
    request_start: bool,
    mut runtime: Option<(&ServiceRuntime, &mut mpsc::Receiver<ServiceIntent>)>,
    mut output: Option<&mut capture::OutputWorker>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output_health_tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        enum Accepted {
            Socket(tokio::net::TcpStream),
            Intent(ServiceIntent),
            OutputHealth,
        }
        let accepted = tokio::select! {
            accepted = listener.accept() => Accepted::Socket(accepted?.0),
            intent = async {
                match runtime.as_mut() {
                    Some((_, receiver)) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => Accepted::Intent(intent.ok_or("service intent channel stopped")?),
            signal = tokio::signal::ctrl_c() => {
                signal?;
                println!("Control service stopped. Camera access was not active.");
                return Ok(());
            }
            _ = output_health_tick.tick() => {
                if let Some(worker) = output.as_deref_mut()
                    && let Err(error) = worker.check_health()
                    && let Some((state, _)) = runtime.as_ref()
                {
                    state.set_output_failed(error);
                }
                Accepted::OutputHealth
            }
        };
        let socket = match accepted {
            Accepted::Socket(socket) => socket,
            Accepted::OutputHealth => continue,
            Accepted::Intent(ServiceIntent::RequestStart { operation_id }) => {
                if let Some((state, _)) = &runtime {
                    state.fail(
                        &operation_id,
                        "peer_offline",
                        "trusted phone is not connected",
                    );
                }
                continue;
            }
            Accepted::Intent(ServiceIntent::Stop { operation_id }) => {
                if let Some((state, _)) = &runtime {
                    state.invalidate_stop();
                    state.succeed(&operation_id);
                }
                continue;
            }
            Accepted::Intent(ServiceIntent::ForgetPeer { operation_id }) => {
                if let Some((state, _)) = &runtime {
                    state.invalidate_forget();
                }
                let result = forget_trust(trust_path);
                if let Some((state, _)) = &runtime {
                    match result {
                        Ok(()) => state.succeed(&operation_id),
                        Err(error) => state.fail(&operation_id, "forget_failed", error),
                    }
                }
                return Ok(());
            }
        };
        let connection_result = run_control_connection(
            socket,
            acceptor,
            certificate_der,
            trusted_public_key,
            phone_name,
            trust_path,
            Arc::clone(&listener),
            request_start,
            runtime
                .as_mut()
                .map(|(state, receiver)| (*state, &mut **receiver)),
            output.as_deref_mut(),
        )
        .await;
        if let Some((state, _)) = &runtime
            && !matches!(connection_result, Ok(ControlConnectionOutcome::ForgotPeer))
        {
            state.connection_lost();
        }
        match connection_result {
            Ok(ControlConnectionOutcome::Disconnected) => {
                println!("{phone_name} disconnected; waiting for authenticated reconnect.");
            }
            Ok(ControlConnectionOutcome::CaptureStopped) => {
                // This outcome is consumed inside run_control_connection; it
                // should never escape the authenticated connection loop.
            }
            Ok(ControlConnectionOutcome::ForgotPeer) => {
                println!("Remote revocation completed; control service is stopping.");
                return Ok(());
            }
            Ok(ControlConnectionOutcome::Shutdown) => return Ok(()),
            Err(error) => {
                eprintln!("Rejected control connection: {error}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_control_connection(
    socket: tokio::net::TcpStream,
    acceptor: &TlsAcceptor,
    certificate_der: &[u8],
    trusted_public_key: &[u8],
    phone_name: &str,
    trust_path: &Path,
    listener: Arc<TcpListener>,
    request_start: bool,
    mut runtime: Option<(&ServiceRuntime, &mut mpsc::Receiver<ServiceIntent>)>,
    mut output: Option<&mut capture::OutputWorker>,
) -> Result<ControlConnectionOutcome, Box<dyn std::error::Error>> {
    let tls_stream = timeout(CLIENT_STEP_TIMEOUT, acceptor.accept(socket))
        .await
        .map_err(|_| "TLS handshake timed out")??;
    let started = Instant::now();
    let mut session = ControlSession::create(certificate_der, 0)?;
    let mut stream = BufReader::new(tls_stream);
    write_json_line(stream.get_mut(), session.hello()).await?;
    let proof: ControlProof = read_json_line(&mut stream).await?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let authenticated = session.authenticate(&proof, trusted_public_key, elapsed_ms)?;
    write_json_line(
        stream.get_mut(),
        &ControlReply::Authenticated {
            version: authenticated.version,
            capture_authorized: false,
        },
    )
    .await?;
    println!("Authenticated {phone_name}. Camera access was not started.");

    let connection_id_bytes = URL_SAFE_NO_PAD
        .decode(&authenticated.session_id)
        .map_err(|_| "authenticated connection identifier is malformed")?;
    let connection_id = ControlConnectionId::new(
        connection_id_bytes
            .try_into()
            .map_err(|_| "authenticated connection identifier has the wrong length")?,
    );
    let peer_identity = PeerIdentity::new(authenticated.phone_key_sha256);
    let tls_stream = stream.into_inner();
    let (control_read, mut control_write) = tokio::io::split(tls_stream);
    let (command_tx, mut command_rx) = mpsc::channel(32);
    tokio::spawn(async move {
        let mut reader = BufReader::new(control_read);
        loop {
            let command = read_json_line::<ControlCommand, _>(&mut reader)
                .await
                .map_err(|error| error.to_string());
            if command_tx.send(command).await.is_err() {
                break;
            }
        }
    });

    let mut policy = SessionPolicy::default();
    policy.trust_peer();
    policy.authenticate_connection()?;
    if output
        .as_deref_mut()
        .is_some_and(capture::OutputWorker::is_healthy)
    {
        policy.set_output_ready();
    }
    if let Some((state, _)) = &runtime {
        state.publish_policy(&policy);
    }
    let mut request_operation_id = None;
    let mut request_id = if request_start {
        policy.request_start()?;
        let value = capture::random_identifier::<16>()?;
        write_json_line(
            &mut control_write,
            &ControlReply::StartRequest {
                request_id: URL_SAFE_NO_PAD.encode(value),
                desktop_name: hostname_display_name(),
                width: 1280,
                height: 720,
                fps: 30,
                codec: "h264-constrained-baseline",
            },
        )
        .await?;
        Some(value)
    } else {
        None
    };
    let mut output_health_tick = tokio::time::interval(std::time::Duration::from_millis(250));

    loop {
        enum Next {
            Phone(ControlCommand),
            Service(ServiceIntent),
            OutputHealth,
        }
        let next = tokio::select! {
            command = command_rx.recv() => Next::Phone(command
                .ok_or("authenticated control reader stopped")??),
            intent = async {
                match runtime.as_mut() {
                    Some((_, receiver)) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => Next::Service(intent.ok_or("service intent channel stopped")?),
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(ControlConnectionOutcome::Shutdown);
            }
            _ = output_health_tick.tick() => {
                if let Some(worker) = output.as_deref_mut()
                    && let Err(error) = worker.check_health()
                    && let Some((state, _)) = runtime.as_ref()
                {
                    state.set_output_failed(error);
                }
                Next::OutputHealth
            }
        };
        if matches!(next, Next::OutputHealth) {
            continue;
        }
        if let Next::Service(intent) = next {
            match intent {
                ServiceIntent::RequestStart { operation_id } => {
                    if request_id.is_some() {
                        if let Some((state, _)) = &runtime {
                            state.fail(&operation_id, "capture_busy", "another Start is pending");
                        }
                        continue;
                    }
                    if let Err(error) = policy.request_start() {
                        if let Some((state, _)) = &runtime {
                            state.fail(&operation_id, "start_rejected", error.to_string());
                        }
                        continue;
                    }
                    let value = capture::random_identifier::<16>()?;
                    write_json_line(
                        &mut control_write,
                        &ControlReply::StartRequest {
                            request_id: URL_SAFE_NO_PAD.encode(value),
                            desktop_name: hostname_display_name(),
                            width: 1280,
                            height: 720,
                            fps: 30,
                            codec: "h264-constrained-baseline",
                        },
                    )
                    .await?;
                    request_id = Some(value);
                    request_operation_id = Some(operation_id);
                    if let Some((state, _)) = &runtime {
                        state.publish_policy(&policy);
                    }
                }
                ServiceIntent::Stop { operation_id } => {
                    policy.stop();
                    request_id = None;
                    request_operation_id = None;
                    if let Some((state, _)) = &runtime {
                        state.publish_policy(&policy);
                        state.succeed(&operation_id);
                    }
                    let _ = write_json_line(
                        &mut control_write,
                        &ControlReply::StopCapture {
                            reason: "desktop requested Stop",
                        },
                    )
                    .await;
                }
                ServiceIntent::ForgetPeer { operation_id } => {
                    policy.forget_peer();
                    if let Some((state, _)) = &runtime {
                        state.publish_policy(&policy);
                    }
                    match forget_trust(trust_path) {
                        Ok(()) => {
                            if let Some((state, _)) = &runtime {
                                state.succeed(&operation_id);
                            }
                        }
                        Err(error) => {
                            if let Some((state, _)) = &runtime {
                                state.fail(&operation_id, "forget_failed", error);
                            }
                        }
                    }
                    return Ok(ControlConnectionOutcome::ForgotPeer);
                }
            }
            continue;
        }
        let Next::Phone(command) = next else {
            unreachable!()
        };
        if !trust_record_matches(trust_path, trusted_public_key) {
            policy.forget_peer();
            return Err("trusted phone was revoked while control was connected".into());
        }
        match command {
            ControlCommand::Ping => {
                write_json_line(
                    &mut control_write,
                    &ControlReply::Pong {
                        capture_authorized: false,
                    },
                )
                .await?;
            }
            ControlCommand::Disconnect => return Ok(ControlConnectionOutcome::Disconnected),
            ControlCommand::ForgetPeer => {
                policy.forget_peer();
                forget_trust(trust_path).map_err(|error| format!("revocation failed: {error}"))?;
                let _ = write_json_line(&mut control_write, &ControlReply::Forgotten).await;
                return Ok(ControlConnectionOutcome::ForgotPeer);
            }
            ControlCommand::StartRejected {
                request_id: rejected,
            } => {
                capture::require_request_id(request_id, &rejected)?;
                policy.stop();
                if let (Some(operation_id), Some((state, _))) =
                    (request_operation_id.take(), &runtime)
                {
                    state.publish_policy(&policy);
                    state.fail(
                        &operation_id,
                        "phone_declined",
                        "phone declined camera sharing",
                    );
                }
                request_id = None;
                println!("{phone_name} declined camera sharing. Camera access was not started.");
            }
            ControlCommand::StartApproved {
                request_id: approved,
            } => {
                capture::require_request_id(request_id, &approved)?;
                let generation = policy.grant_consent(capture::monotonic_millis())?;
                if let Some((state, _)) = &runtime {
                    state.publish_policy(&policy);
                }
                let binding = MediaBinding {
                    peer_identity,
                    control_connection_id: connection_id,
                    media_session_id: MediaSessionId::new(capture::random_identifier()?),
                    generation,
                };
                write_json_line(
                    &mut control_write,
                    &ControlReply::CaptureGranted {
                        request_id: approved,
                        peer: peer_identity.to_hex(),
                        connection: connection_id.to_hex(),
                        session: binding.media_session_id.to_hex(),
                        generation,
                    },
                )
                .await?;
                let capture_outcome = capture::run_active_capture(
                    Arc::clone(&listener),
                    acceptor,
                    certificate_der,
                    trusted_public_key,
                    trust_path,
                    &mut control_write,
                    &mut command_rx,
                    &mut policy,
                    binding,
                    output.as_deref_mut(),
                    runtime
                        .as_mut()
                        .map(|(state, receiver)| (*state, &mut **receiver)),
                    request_operation_id.take(),
                )
                .await;
                match capture_outcome? {
                    ControlConnectionOutcome::CaptureStopped => {
                        request_id = None;
                        request_operation_id = None;
                    }
                    outcome => return Ok(outcome),
                }
            }
            ControlCommand::Stop | ControlCommand::MediaOpen { .. } => {
                policy.stop();
                return Err("capture command arrived outside an active session".into());
            }
        }
    }
}

pub(crate) fn trust_record_matches(path: &Path, expected_public_key: &[u8]) -> bool {
    load_trust_record(path)
        .ok()
        .and_then(|record| {
            URL_SAFE_NO_PAD
                .decode(record.phone_public_key_der_base64url)
                .ok()
        })
        .is_some_and(|public_key| public_key == expected_public_key)
}

fn advertise_control_service(
    listen: SocketAddr,
    certificate_digest: &str,
) -> Result<(ServiceDaemon, String), Box<dyn std::error::Error>> {
    let discovery = ServiceDaemon::new()?;
    let suffix = certificate_digest
        .get(..12)
        .ok_or("certificate digest is unexpectedly short")?;
    let instance_name = format!("OmaCam-{suffix}");
    let host_name = format!("omacam-{suffix}.local.");
    let properties = [
        ("v", "1"),
        ("cert", certificate_digest),
        ("role", "desktop-control"),
    ];
    let service = ServiceInfo::new(
        "_omacam._tcp.local.",
        &instance_name,
        &host_name,
        listen.ip(),
        listen.port(),
        &properties[..],
    )?;
    let fullname = service.get_fullname().to_owned();
    discovery.register(service)?;
    Ok((discovery, fullname))
}

pub(crate) async fn read_json_line<
    T: serde::de::DeserializeOwned,
    R: tokio::io::AsyncRead + Unpin,
>(
    reader: &mut BufReader<R>,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut message = Vec::new();
    let bytes_read = timeout(
        CLIENT_STEP_TIMEOUT,
        reader
            .take(MAX_CONTROL_MESSAGE_BYTES + 1)
            .read_until(b'\n', &mut message),
    )
    .await
    .map_err(|_| "control message timed out")??;
    if bytes_read == 0 || bytes_read as u64 > MAX_CONTROL_MESSAGE_BYTES {
        return Err("control message is empty or too large".into());
    }
    if message.last() == Some(&b'\n') {
        message.pop();
    }
    Ok(serde_json::from_slice(&message)?)
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ControlReply {
    Authenticated {
        version: u8,
        capture_authorized: bool,
    },
    Pong {
        capture_authorized: bool,
    },
    StartRequest {
        request_id: String,
        desktop_name: String,
        width: u16,
        height: u16,
        fps: u8,
        codec: &'static str,
    },
    CaptureGranted {
        request_id: String,
        peer: String,
        connection: String,
        session: String,
        generation: u64,
    },
    MediaReady,
    Stopped {
        reason: String,
    },
    StopCapture {
        reason: &'static str,
    },
    Forgotten,
}

pub(crate) enum ControlCommand {
    Ping,
    Disconnect,
    ForgetPeer,
    StartApproved {
        request_id: String,
    },
    StartRejected {
        request_id: String,
    },
    Stop,
    MediaOpen {
        peer: String,
        connection: String,
        media_session: String,
        generation: u64,
    },
}

impl<'de> Deserialize<'de> for ControlCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(D::Error::custom)
    }
}

impl ControlCommand {
    fn from_value(value: &serde_json::Value) -> Result<Self, &'static str> {
        let object = value
            .as_object()
            .ok_or("control command must be an object")?;
        let message_type = object
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or("control command type is missing")?;
        let exact = |keys: &[&str]| {
            object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
        };
        let request_id = || {
            object
                .get("request_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or("Start request identifier is missing")
        };
        match message_type {
            "ping" if exact(&["type"]) => Ok(Self::Ping),
            "disconnect" if exact(&["type"]) => Ok(Self::Disconnect),
            "forget_peer" if exact(&["type"]) => Ok(Self::ForgetPeer),
            "stop" if exact(&["type"]) => Ok(Self::Stop),
            "start_approved" if exact(&["type", "request_id"]) => Ok(Self::StartApproved {
                request_id: request_id()?,
            }),
            "start_rejected" if exact(&["type", "request_id"]) => Ok(Self::StartRejected {
                request_id: request_id()?,
            }),
            "media_open" if exact(&["type", "peer", "connection", "session", "generation"]) => {
                let text = |key| {
                    object
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .ok_or("media binding text field is missing")
                };
                Ok(Self::MediaOpen {
                    peer: text("peer")?,
                    connection: text("connection")?,
                    media_session: text("session")?,
                    generation: object
                        .get("generation")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or("media generation is invalid")?,
                })
            }
            _ => Err("control command fields are not recognized"),
        }
    }
}

pub(crate) async fn write_json_line<T: Serialize, W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    reply: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_vec(reply)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_cli_requires_explicit_start_and_output_as_one_operation() {
        let base = vec!["--listen".to_owned(), "192.168.50.10:47123".to_owned()];
        assert!(ControlServeOptions::parse(&base).is_ok());

        let mut start_only = base.clone();
        start_only.push("--request-start".to_owned());
        assert!(ControlServeOptions::parse(&start_only).is_err());

        let mut complete = start_only;
        complete.extend(["--output-device".to_owned(), "/dev/video42".to_owned()]);
        assert!(ControlServeOptions::parse(&complete).is_ok());

        let mut preview_without_capture = base;
        preview_without_capture.extend([
            "--preview-socket".to_owned(),
            "/run/user/1000/omacam/preview.sock".to_owned(),
        ]);
        assert!(ControlServeOptions::parse(&preview_without_capture).is_err());

        complete.extend([
            "--preview-socket".to_owned(),
            "/run/user/1000/omacam/preview.sock".to_owned(),
        ]);
        assert!(ControlServeOptions::parse(&complete).is_ok());
    }

    #[test]
    fn control_commands_reject_unknown_fields_and_malformed_media_binding() {
        let unknown = br#"{"type":"ping","capture_authorized":true}"#;
        assert!(serde_json::from_slice::<ControlCommand>(unknown).is_err());

        let malformed = br#"{"type":"media_open","peer":"short","connection":"x","session":"y","generation":1}"#;
        let command: ControlCommand = serde_json::from_slice(malformed).expect("shape parses");
        let ControlCommand::MediaOpen {
            peer,
            connection,
            media_session,
            ..
        } = command
        else {
            panic!("expected media_open");
        };
        assert!(PeerIdentity::from_hex(&peer).is_err());
        assert!(ControlConnectionId::from_hex(&connection).is_err());
        assert!(MediaSessionId::from_hex(&media_session).is_err());
    }

    #[test]
    fn capture_grant_echoes_the_exact_approved_request() {
        let reply = ControlReply::CaptureGranted {
            request_id: "request-token".to_owned(),
            peer: "11".repeat(32),
            connection: "22".repeat(16),
            session: "33".repeat(16),
            generation: 7,
        };
        let encoded = serde_json::to_value(reply).expect("serialize capture grant");

        assert_eq!(encoded["type"], "capture_granted");
        assert_eq!(encoded["request_id"], "request-token");
    }
}
