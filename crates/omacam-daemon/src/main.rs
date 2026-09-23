use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::{self, Write as IoWrite};
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use omacam_core::SessionPolicy;
use omacam_core::pairing::{
    ApprovalSide, INVITATION_LIFETIME_SECS, InvitationState, PairingClaim, PairingInvitation,
    PairingSession, VerifiedClaim,
};
use qrcode::QrCode;
use qrcode::render::{svg, unicode};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::net::TcpListener;
use tokio::time::{Instant, timeout};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::PrivateKeyDer;

mod browser_server;
mod capture;
mod control_server;
mod diagnostics;
mod local_api;
mod service_runtime;

use control_server::{ControlServeOptions, run_control_server, write_json_line};
#[cfg(test)]
use diagnostics::{describe_uvc_candidate, reports_capture_only};
use diagnostics::{doctor_report, provider_report};
use local_api::{run_ipc_client, run_service};

const USAGE: &str = "Usage:
  omacam-daemon doctor
  omacam-daemon providers
  omacam-daemon snapshot
  omacam-daemon service
  omacam-daemon ipc snapshot
  omacam-daemon ipc diagnostics
  omacam-daemon ipc events
  omacam-daemon ipc start|stop|forget|diagnostics-intent <OPERATION_ID>
  omacam-daemon browser serve --endpoint <LAN_IP:HTTPS_PORT> --output-device /dev/videoN [--qr <SVG_PATH>] [--identity <JSON_PATH>]
  omacam-daemon pair serve --endpoint <LAN_IP:PORT> [--listen <IP:PORT>] [--name <NAME>] [--qr <SVG_PATH>] [--trust <JSON_PATH>] [--identity <JSON_PATH>]
  omacam-daemon pair status [--trust <JSON_PATH>]
  omacam-daemon pair forget [--trust <JSON_PATH>]
  omacam-daemon control serve --listen <IP:PORT> [--request-start --output-device /dev/videoN [--preview-socket /owned/runtime/preview.sock]] [--trust <JSON_PATH>] [--identity <JSON_PATH>]";
const MAX_CONTROL_MESSAGE_BYTES: u64 = 65_536;
const CLIENT_STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PAIRING_FAILURES_PER_SOURCE: u8 = 5;
const MAX_PAIRING_FAILURES_GLOBAL: u8 = 20;

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let exit_code = match args.first().map(String::as_str) {
        Some("doctor") => {
            print!("{}", doctor_report());
            0
        }
        Some("providers") => {
            print!("{}", provider_report());
            0
        }
        Some("snapshot") => {
            println!("{:?}", SessionPolicy::default().snapshot());
            0
        }
        Some("service") => match run_service(&args[1..]).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("OmaCam service failed: {error}");
                1
            }
        },
        Some("ipc") => match run_ipc_client(&args[1..]).await {
            Ok(output) => {
                println!("{output}");
                0
            }
            Err(error) => {
                eprintln!("OmaCam service unavailable: {error}");
                1
            }
        },
        Some("browser") if args.get(1).map(String::as_str) == Some("serve") => {
            match browser_server::BrowserServeOptions::parse(&args[2..]) {
                Ok(options) => match browser_server::run(options).await {
                    Ok(()) => 0,
                    Err(error) => {
                        eprintln!("Browser camera failed: {error}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!("{error}\n\n{USAGE}");
                    2
                }
            }
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            0
        }
        Some("pair") if args.get(1).map(String::as_str) == Some("serve") => {
            match PairServeOptions::parse(&args[2..]).and_then(|options| {
                validate_pair_paths(&options)?;
                Ok(options)
            }) {
                Ok(options) => match run_pair_server(options).await {
                    Ok(()) => 0,
                    Err(error) => {
                        eprintln!("Pairing failed: {error}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!("{error}\n\n{USAGE}");
                    2
                }
            }
        }
        Some("pair") if args.get(1).map(String::as_str) == Some("status") => {
            match trust_path_argument(&args[2..]).and_then(|path| show_trust_status(&path)) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Some("pair") if args.get(1).map(String::as_str) == Some("forget") => {
            match trust_path_argument(&args[2..]).and_then(|path| forget_trust(&path)) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Some("control") if args.get(1).map(String::as_str) == Some("serve") => {
            match ControlServeOptions::parse(&args[2..]) {
                Ok(options) => match run_control_server(&options).await {
                    Ok(()) => 0,
                    Err(error) => {
                        eprintln!("Control authentication failed: {error}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!("{error}\n\n{USAGE}");
                    2
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };

    std::process::exit(exit_code);
}

#[derive(Debug)]
struct PairServeOptions {
    endpoint: SocketAddr,
    listen: SocketAddr,
    desktop_name: String,
    qr_path: std::path::PathBuf,
    trust_path: std::path::PathBuf,
    identity_path: std::path::PathBuf,
    auto_approve: bool,
}

impl PairServeOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut endpoint = None;
        let mut listen = None;
        let mut desktop_name = hostname_display_name();
        let mut qr_path = default_runtime_path("pairing.svg");
        let mut trust_path = default_data_path("trusted-phone.json");
        let mut identity_path = default_data_path("desktop-identity.json");
        let mut auto_approve = false;
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            if flag == "--auto-approve" {
                auto_approve = true;
                index += 1;
                continue;
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag {
                "--endpoint" => {
                    endpoint =
                        Some(value.parse::<SocketAddr>().map_err(|_| {
                            "--endpoint must be IP:PORT (use [IPv6]:PORT)".to_owned()
                        })?);
                }
                "--listen" => {
                    listen =
                        Some(value.parse::<SocketAddr>().map_err(|_| {
                            "--listen must be IP:PORT (use [IPv6]:PORT)".to_owned()
                        })?);
                }
                "--name" => desktop_name.clone_from(value),
                "--qr" => qr_path = value.into(),
                "--trust" => trust_path = value.into(),
                "--identity" => identity_path = value.into(),
                _ => return Err(format!("unknown pairing option: {flag}")),
            }
            index += 2;
        }
        let endpoint = endpoint.ok_or_else(|| "--endpoint is required".to_owned())?;
        let listen =
            listen.unwrap_or_else(|| SocketAddr::new(IpAddr::from([0, 0, 0, 0]), endpoint.port()));
        Ok(Self {
            endpoint,
            listen,
            desktop_name,
            qr_path,
            trust_path,
            identity_path,
            auto_approve,
        })
    }
}

fn validate_pair_paths(options: &PairServeOptions) -> Result<(), String> {
    if options.qr_path == options.trust_path {
        return Err("QR and trust-record paths must be different".to_owned());
    }
    if options.trust_path.exists() {
        return Err(format!(
            "a phone is already trusted at {}; forget it explicitly before replacement",
            options.trust_path.display()
        ));
    }
    Ok(())
}

async fn run_pair_server(options: PairServeOptions) -> Result<(), Box<dyn std::error::Error>> {
    let identity = load_or_create_identity(&options.identity_path)?;
    let certificate_der =
        tokio_rustls::rustls::pki_types::CertificateDer::from(identity.certificate_der.clone());
    let private_key = PrivateKeyDer::try_from(identity.private_key_der)?;
    let tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate_der.clone()], private_key)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let now_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let invitation = PairingInvitation::create(
        options.endpoint,
        &options.desktop_name,
        certificate_der.as_ref(),
        now_unix_secs,
    )?;
    let payload = invitation.expose_qr_payload()?;
    write_qr_svg(&options.qr_path, &payload)?;
    let qr_terminal = QrCode::new(payload.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .build();

    println!("OmaCam secure pairing");
    println!("Scan this QR inside the OmaCam Android companion:\n{qr_terminal}");
    println!("QR image: {}", options.qr_path.display());
    println!("Waiting for a phone for {INVITATION_LIFETIME_SECS} seconds…");

    let listener = Arc::new(TcpListener::bind(options.listen).await?);
    let started = Instant::now();
    let mut session = PairingSession::new(invitation, 0);
    let outcome = tokio::select! {
        result = run_pairing_ceremony(
            &listener,
            &acceptor,
            &started,
            &mut session,
            &options,
        ) => result,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            Err("pairing cancelled".into())
        }
    };

    if let Err(error) = std::fs::remove_file(&options.qr_path)
        && error.kind() != io::ErrorKind::NotFound
    {
        eprintln!("Warning: could not remove expired QR image: {error}");
    }
    outcome
}

type PairStream = BufReader<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

async fn wait_for_valid_claim(
    listener: &TcpListener,
    acceptor: &TlsAcceptor,
    started: &Instant,
    session: &mut PairingSession,
) -> Result<(PairStream, VerifiedClaim), Box<dyn std::error::Error>> {
    let mut source_failures = HashMap::<IpAddr, u8>::new();
    let mut global_failures = 0_u8;
    loop {
        let remaining =
            Duration::from_secs(INVITATION_LIFETIME_SECS).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("invitation expired".into());
        }
        let (socket, peer) = timeout(remaining, listener.accept())
            .await
            .map_err(|_| "invitation expired")??;
        if source_failures
            .get(&peer.ip())
            .is_some_and(|failures| *failures >= MAX_PAIRING_FAILURES_PER_SOURCE)
        {
            continue;
        }
        let Ok(Ok(tls_stream)) = timeout(CLIENT_STEP_TIMEOUT, acceptor.accept(socket)).await else {
            record_pairing_failure(&mut source_failures, &mut global_failures, peer.ip())?;
            continue;
        };
        let mut stream = BufReader::new(tls_stream);
        let mut message = Vec::new();
        let read = timeout(
            CLIENT_STEP_TIMEOUT,
            (&mut stream)
                .take(MAX_CONTROL_MESSAGE_BYTES + 1)
                .read_until(b'\n', &mut message),
        )
        .await;
        let Ok(Ok(bytes_read)) = read else {
            record_pairing_failure(&mut source_failures, &mut global_failures, peer.ip())?;
            continue;
        };
        if bytes_read == 0 || bytes_read as u64 > MAX_CONTROL_MESSAGE_BYTES {
            record_pairing_failure(&mut source_failures, &mut global_failures, peer.ip())?;
            continue;
        }
        if message.last() == Some(&b'\n') {
            message.pop();
        }
        let Ok(claim) = serde_json::from_slice::<PairingClaim>(&message) else {
            write_json_line(
                stream.get_mut(),
                &ServerReply::Rejected {
                    reason: "malformed claim",
                },
            )
            .await?;
            record_pairing_failure(&mut source_failures, &mut global_failures, peer.ip())?;
            continue;
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let Ok(verified) = session.claim(&claim, elapsed_ms) else {
            write_json_line(
                stream.get_mut(),
                &ServerReply::Rejected {
                    reason: "claim rejected",
                },
            )
            .await?;
            record_pairing_failure(&mut source_failures, &mut global_failures, peer.ip())?;
            continue;
        };
        return Ok((stream, verified.clone()));
    }
}

fn record_pairing_failure(
    source_failures: &mut HashMap<IpAddr, u8>,
    global_failures: &mut u8,
    source: IpAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let failures = source_failures.entry(source).or_default();
    *failures = failures.saturating_add(1);
    *global_failures = global_failures.saturating_add(1);
    if *global_failures >= MAX_PAIRING_FAILURES_GLOBAL {
        return Err("pairing invitation rejected after too many failed attempts".into());
    }
    Ok(())
}

async fn run_pairing_ceremony(
    listener: &TcpListener,
    acceptor: &TlsAcceptor,
    started: &Instant,
    session: &mut PairingSession,
    options: &PairServeOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, verified) = wait_for_valid_claim(listener, acceptor, started, session).await?;
    session.approve(ApprovalSide::Phone)?;
    write_json_line(
        stream.get_mut(),
        &ServerReply::Confirm {
            desktop_name: &options.desktop_name,
            phone_name: &verified.phone_name,
            sas: &verified.sas,
        },
    )
    .await?;

    println!("\nPhone: {}", verified.phone_name);
    println!("Confirmation code: {}", verified.sas);
    println!("Approve only if the phone shows the same code.");
    let expected = verified.sas.clone();
    let entered = if options.auto_approve {
        println!("Auto-approving pairing code {expected}");
        true
    } else {
        tokio::task::spawn_blocking(move || prompt_for_code(&expected)).await??
    };
    if !entered {
        session.reject();
        write_json_line(
            stream.get_mut(),
            &ServerReply::Rejected {
                reason: "desktop rejected",
            },
        )
        .await?;
        return Err("desktop rejected pairing".into());
    }
    let state = session.approve(ApprovalSide::Desktop)?;
    if state != InvitationState::Paired {
        return Err("both approvals were not recorded".into());
    }
    persist_trust_record(&options.trust_path, &verified)?;
    write_json_line(stream.get_mut(), &ServerReply::Paired).await?;
    println!(
        "Paired with {}. Camera access was not started.",
        verified.phone_name
    );
    Ok(())
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerReply<'a> {
    Confirm {
        desktop_name: &'a str,
        phone_name: &'a str,
        sas: &'a str,
    },
    Paired,
    Rejected {
        reason: &'a str,
    },
}

fn write_qr_svg(path: &Path, payload: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        create_private_parent(parent)?;
    }
    let image = QrCode::new(payload.as_bytes())?
        .render::<svg::Color>()
        .min_dimensions(512, 512)
        .quiet_zone(true)
        .build();
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(image.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn prompt_for_code(expected: &str) -> io::Result<bool> {
    print!("Type the six-digit code to approve (or press Enter to reject): ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim() == expected)
}

#[derive(Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustRecord {
    schema_version: u8,
    phone_name: String,
    phone_public_key_der_base64url: String,
    phone_key_sha256_base64url: String,
}

fn load_trust_record(path: &Path) -> Result<TrustRecord, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Err("no trusted phone; pair before starting control".into());
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err("trust record must be a private non-symlink regular file".into());
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > 16_384 {
        return Err("trust record exceeds size limit".into());
    }
    let record: TrustRecord = serde_json::from_slice(&bytes)?;
    if record.schema_version != 1 {
        return Err("unsupported trust-record version".into());
    }
    Ok(record)
}

#[derive(Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDesktopIdentity {
    version: u8,
    certificate_der: Vec<u8>,
    private_key_der: Vec<u8>,
}

fn load_or_create_identity(
    path: &Path,
) -> Result<StoredDesktopIdentity, Box<dyn std::error::Error>> {
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || metadata.permissions().mode() & 0o077 != 0 {
            return Err("desktop identity must be a non-symlink file with mode 0600".into());
        }
        let bytes = std::fs::read(path)?;
        if bytes.len() > 16_384 {
            return Err("desktop identity file exceeds size limit".into());
        }
        let identity: StoredDesktopIdentity = serde_json::from_slice(&bytes)?;
        if identity.version != 1
            || identity.certificate_der.is_empty()
            || identity.certificate_der.len() > 8_192
            || identity.private_key_der.is_empty()
            || identity.private_key_der.len() > 8_192
        {
            return Err("desktop identity file is invalid".into());
        }
        return Ok(identity);
    }

    let parent = path.parent().ok_or("desktop identity path has no parent")?;
    create_private_parent(parent)?;
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["omacam.local".to_owned()])?;
    let identity = StoredDesktopIdentity {
        version: 1,
        certificate_der: cert.der().to_vec(),
        private_key_der: signing_key.serialize_der(),
    };
    let encoded = serde_json::to_vec(&identity)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    Ok(identity)
}

fn persist_trust_record(
    path: &Path,
    claim: &omacam_core::pairing::VerifiedClaim,
) -> Result<(), Box<dyn std::error::Error>> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let parent = path.parent().ok_or("trust path has no parent directory")?;
    create_private_parent(parent)?;
    let temporary = path.with_extension("json.new");
    let record = TrustRecord {
        schema_version: 1,
        phone_name: claim.phone_name.clone(),
        phone_public_key_der_base64url: URL_SAFE_NO_PAD.encode(&claim.phone_public_key_der),
        phone_key_sha256_base64url: URL_SAFE_NO_PAD.encode(claim.phone_key_sha256),
    };
    let encoded = serde_json::to_vec_pretty(&record)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn trust_path_argument(args: &[String]) -> Result<std::path::PathBuf, String> {
    match args {
        [] => Ok(default_data_path("trusted-phone.json")),
        [flag, path] if flag == "--trust" => Ok(path.into()),
        _ => Err("expected no options or exactly --trust <JSON_PATH>".to_owned()),
    }
}

fn show_trust_status(path: &Path) -> Result<(), String> {
    if !path.exists() {
        println!("No phone is trusted.");
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("trust record must be a private regular file".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() > 16_384 {
        return Err("trust record exceeds size limit".to_owned());
    }
    let record: TrustRecord = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if record.schema_version != 1 {
        return Err("unsupported trust-record version".to_owned());
    }
    println!("Trusted phone: {}", record.phone_name);
    println!("Identity: {}", record.phone_key_sha256_base64url);
    Ok(())
}

fn forget_trust(path: &Path) -> Result<(), String> {
    if !path.exists() {
        println!("No phone was trusted.");
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err("refusing to remove a non-regular trust path".to_owned());
    }
    std::fs::remove_file(path).map_err(|error| error.to_string())?;
    println!("Forgot the trusted phone. The desktop identity was retained.");
    Ok(())
}

fn create_private_parent(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn hostname_display_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "OmaCam desktop".to_owned())
}

fn default_runtime_path(file_name: &str) -> std::path::PathBuf {
    env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || std::path::PathBuf::from("/tmp/omacam").join(file_name),
        |root| {
            std::path::PathBuf::from(root)
                .join("omacam")
                .join(file_name)
        },
    )
}

fn default_data_path(file_name: &str) -> std::path::PathBuf {
    env::var_os("XDG_DATA_HOME").map_or_else(
        || {
            env::var_os("HOME").map_or_else(
                || std::path::PathBuf::from(".").join(file_name),
                |home| {
                    std::path::PathBuf::from(home)
                        .join(".local/share/omacam")
                        .join(file_name)
                },
            )
        },
        |root| {
            std::path::PathBuf::from(root)
                .join("omacam")
                .join(file_name)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("omacam-{label}-{}", std::process::id()))
    }

    #[test]
    fn doctor_report_explains_that_it_does_not_repair() {
        let report = doctor_report();

        assert!(report.starts_with("OmaCam host readiness (read-only)"));
        assert!(report.contains("no repair was attempted"));
        assert!(report.contains("v4l2loopback module"));
        assert!(report.contains("OmaCam output writer"));
    }

    #[test]
    fn capture_only_caps_mean_a_writer_is_attached() {
        assert!(reports_capture_only(
            "Capabilities\n  Video Capture\n  Streaming"
        ));
        assert!(!reports_capture_only(
            "Capabilities\n  Video Capture\n  Video Output\n  Streaming"
        ));
    }

    #[test]
    fn desktop_identity_is_private_and_stable() {
        let directory = test_directory("identity-test");
        let path = directory.join("identity.json");
        let first = load_or_create_identity(&path).expect("create identity");
        let second = load_or_create_identity(&path).expect("reload identity");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600);
        assert_eq!(first.certificate_der, second.certificate_der);
        assert_eq!(first.private_key_der, second.private_key_der);
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn qr_file_is_private_and_cannot_be_overwritten() {
        let directory = test_directory("qr-test");
        let path = directory.join("pairing.svg");
        write_qr_svg(&path, "bounded test payload").expect("write QR");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600);
        assert!(write_qr_svg(&path, "replacement").is_err());
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn pairing_failure_quota_is_bounded_globally_and_per_source() {
        let mut sources = HashMap::new();
        let mut global = 0;
        let source: IpAddr = "192.168.50.20".parse().expect("test IP");

        for _ in 0..MAX_PAIRING_FAILURES_PER_SOURCE {
            record_pairing_failure(&mut sources, &mut global, source).expect("below global quota");
        }

        assert_eq!(sources[&source], MAX_PAIRING_FAILURES_PER_SOURCE);
        for suffix in 21..35 {
            let other: IpAddr = format!("192.168.50.{suffix}").parse().expect("test IP");
            record_pairing_failure(&mut sources, &mut global, other).expect("below global quota");
        }
        let final_source: IpAddr = "192.168.50.35".parse().expect("test IP");
        assert!(record_pairing_failure(&mut sources, &mut global, final_source).is_err());
        assert_eq!(global, MAX_PAIRING_FAILURES_GLOBAL);
    }

    #[test]
    fn provider_detection_accepts_uvc_and_rejects_unrelated_video_devices() {
        let uvc = "ID_USB_DRIVER=uvcvideo\nID_VENDOR=OnePlus\nID_MODEL=Phone_Camera\n";
        let unrelated = "ID_VENDOR=Virtual\nID_MODEL=Loopback\n";

        assert_eq!(
            describe_uvc_candidate("video2", uvc),
            Some("/dev/video2: OnePlus — Phone_Camera".to_owned())
        );
        assert_eq!(describe_uvc_candidate("video42", unrelated), None);
    }
}
