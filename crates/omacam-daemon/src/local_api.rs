//! Versioned, bounded local session-bus API owned by the real session runtime.
use crate::control_server::{ControlServeOptions, run_owned_control_server};
use crate::diagnostics::{doctor_report, omacam_output_ready, provider_report};
use crate::load_trust_record;
use crate::service_runtime::{CaptureStatsSnapshot, OperationSnapshot, ServiceRuntime};
use futures_util::StreamExt as _;
use omacam_core::SessionPolicy;
use omacam_core::control::CONTROL_PROTOCOL_VERSION;
use serde::Serialize;
use zbus::{Connection, connection, interface, object_server::SignalEmitter};

pub(crate) const BUS_NAME: &str = "dev.omacam.Session";
pub(crate) const OBJECT_PATH: &str = "/dev/omacam/Session";
pub(crate) const INTERFACE_NAME: &str = "dev.omacam.Session1";
const API_VERSION: u16 = 1;
const SNAPSHOT_SCHEMA_VERSION: u16 = 2;
const MAX_REPLY_BYTES: usize = 65_536;

#[derive(Debug, Serialize)]
struct LocalSnapshot {
    schema_version: u16,
    api_version: u16,
    revision: u64,
    protocol_version: u8,
    state: omacam_core::SessionSnapshot,
    provider: ProviderSnapshot,
    output: OutputSnapshot,
    preview: PreviewSnapshot,
    capabilities_revision: u64,
    applied_settings: Option<omacam_core::camera::AppliedCameraState>,
    camera_capabilities: Option<omacam_core::camera::CameraCapabilities>,
    stats: Option<CaptureStatsSnapshot>,
    operations: Vec<OperationSnapshot>,
    last_error: Option<OperationSnapshot>,
}
#[derive(Debug, Serialize)]
struct ProviderSnapshot {
    order: [&'static str; 3],
    selected: Option<&'static str>,
    companion: &'static str,
    native_uvc: &'static str,
    browser: &'static str,
}
#[derive(Debug, Serialize)]
struct OutputSnapshot {
    ready: bool,
    mode: &'static str,
}
#[derive(Debug, Serialize)]
struct PreviewSnapshot {
    transport: &'static str,
    source: &'static str,
    active: bool,
}
#[derive(Debug, Serialize)]
struct DiagnosticsSnapshot {
    schema_version: u16,
    revision: u64,
    host: String,
    providers: String,
}
#[derive(Debug, Serialize)]
struct IntentReply {
    operation_id: String,
    status: &'static str,
}

#[derive(Debug)]
struct SessionApi {
    runtime: ServiceRuntime,
}
impl SessionApi {
    fn snapshot_json(&self) -> Result<String, zbus::fdo::Error> {
        bounded_json(&build_snapshot(&self.runtime))
    }
    fn diagnostics_json(&self) -> Result<String, zbus::fdo::Error> {
        bounded_json(&DiagnosticsSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            revision: self.runtime.snapshot().revision,
            host: doctor_report(),
            providers: provider_report(),
        })
    }
    fn accepted(operation_id: String) -> zbus::fdo::Result<String> {
        bounded_json(&IntentReply {
            operation_id,
            status: "accepted",
        })
    }
    fn typed(error: &'static str) -> zbus::fdo::Error {
        zbus::fdo::Error::InvalidArgs(format!("{error}: intent rejected"))
    }
}

#[interface(name = "dev.omacam.Session1")]
impl SessionApi {
    fn get_snapshot(&self) -> zbus::fdo::Result<String> {
        self.snapshot_json()
    }
    async fn camera_control(
        &self,
        operation_id: String,
        generation: u64,
        controls_json: String,
    ) -> zbus::fdo::Result<String> {
        if controls_json.len() > 4096 {
            return Err(Self::typed("controls_too_large"));
        }
        let controls =
            serde_json::from_str(&controls_json).map_err(|_| Self::typed("invalid_controls"))?;
        self.runtime
            .camera_control(operation_id.clone(), generation, controls)
            .await
            .map_err(Self::typed)?;
        Self::accepted(operation_id)
    }
    fn run_diagnostics(&self) -> zbus::fdo::Result<String> {
        self.diagnostics_json()
    }
    async fn request_start(&self, operation_id: String) -> zbus::fdo::Result<String> {
        self.runtime
            .request_start(operation_id.clone())
            .await
            .map_err(Self::typed)?;
        Self::accepted(operation_id)
    }
    async fn stop(&self, operation_id: String) -> zbus::fdo::Result<String> {
        self.runtime
            .stop(operation_id.clone())
            .await
            .map_err(Self::typed)?;
        Self::accepted(operation_id)
    }
    async fn forget_peer(&self, operation_id: String) -> zbus::fdo::Result<String> {
        self.runtime
            .forget_peer(operation_id.clone())
            .await
            .map_err(Self::typed)?;
        Self::accepted(operation_id)
    }
    fn request_diagnostics(&self, operation_id: String) -> zbus::fdo::Result<String> {
        self.runtime
            .run_diagnostics(&operation_id)
            .map_err(Self::typed)?;
        Self::accepted(operation_id)
    }
    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, revision: u64) -> zbus::Result<()>;
}

pub(crate) async fn run_service(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let options = ControlServeOptions::parse_owned(args)?;
    let mut policy = SessionPolicy::default();
    let has_trust = load_trust_record(&options.trust_path).is_ok();
    if has_trust {
        policy.trust_peer();
    }
    if omacam_output_ready() {
        policy.set_output_ready();
    }
    let (runtime, intent_rx) = ServiceRuntime::new(&policy, options.trust_path.clone());
    let connection = connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            SessionApi {
                runtime: runtime.clone(),
            },
        )?
        .build()
        .await?;
    let mut revisions = runtime.subscribe();
    let signal_connection = connection.clone();
    tokio::spawn(async move {
        while revisions.changed().await.is_ok() {
            if let Ok(emitter) = SignalEmitter::new(&signal_connection, OBJECT_PATH) {
                let revision = *revisions.borrow_and_update();
                let _ = SessionApi::state_changed(&emitter, revision).await;
            }
        }
    });
    // Output ownership is service-scoped, not trust- or connection-scoped.  The
    // owned runtime also drains safe lifecycle intents while no peer is paired.
    run_owned_control_server(&options, runtime, intent_rx).await
}

pub(crate) async fn run_ipc_client(args: &[String]) -> Result<String, String> {
    if matches!(args, [command] if command == "events") {
        let connection = Connection::session()
            .await
            .map_err(|error| error.to_string())?;
        let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE_NAME)
            .await
            .map_err(|error| error.to_string())?;
        let mut signals = proxy
            .receive_signal("StateChanged")
            .await
            .map_err(|error| error.to_string())?;
        while let Some(message) = signals.next().await {
            let revision: u64 = message
                .body()
                .deserialize()
                .map_err(|error| error.to_string())?;
            println!("{revision}");
        }
        return Ok(String::new());
    }
    let (method, argument) = match args {
        [command] if command == "snapshot" => ("GetSnapshot", None),
        [command] if command == "diagnostics" => ("RunDiagnostics", None),
        [command, operation_id] if command == "start" => ("RequestStart", Some(operation_id)),
        [command, operation_id] if command == "stop" => ("Stop", Some(operation_id)),
        [command, operation_id] if command == "forget" => ("ForgetPeer", Some(operation_id)),
        [command, operation_id] if command == "diagnostics-intent" => ("RequestDiagnostics", Some(operation_id)),
        _ => return Err("expected ipc snapshot|diagnostics or ipc start|stop|forget|diagnostics-intent <OPERATION_ID>".to_owned()),
    };
    let connection = Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE_NAME)
        .await
        .map_err(|error| error.to_string())?;
    match argument {
        Some(value) => proxy
            .call(method, &(value,))
            .await
            .map_err(|error| error.to_string()),
        None => proxy
            .call(method, &())
            .await
            .map_err(|error| error.to_string()),
    }
}

fn build_snapshot(runtime: &ServiceRuntime) -> LocalSnapshot {
    let current = runtime.snapshot();
    let companion_ready = current.state.trust == omacam_core::TrustState::Trusted;
    LocalSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        api_version: API_VERSION,
        revision: current.revision,
        protocol_version: CONTROL_PROTOCOL_VERSION,
        state: current.state,
        provider: ProviderSnapshot {
            order: ["native_uvc", "browser", "companion"],
            selected: companion_ready.then_some("companion"),
            companion: if companion_ready {
                "ready"
            } else {
                "needs_pairing"
            },
            native_uvc: "requires_confirmation",
            browser: "unqualified",
        },
        output: OutputSnapshot {
            ready: !matches!(
                current.state.output,
                omacam_core::OutputState::Missing | omacam_core::OutputState::Failed
            ),
            mode: "yuy2_1280x720_30",
        },
        preview: PreviewSnapshot {
            transport: "unix_fd_rgbx",
            source: "post_decoder",
            active: current.state.capture == omacam_core::CaptureState::Streaming,
        },
        capabilities_revision: current.revision,
        applied_settings: current.applied_camera,
        camera_capabilities: current.camera_capabilities,
        stats: current.capture_stats,
        operations: current.operations,
        last_error: current.last_error,
    }
}
fn bounded_json<T: Serialize>(value: &T) -> Result<String, zbus::fdo::Error> {
    let encoded = serde_json::to_string(value)
        .map_err(|_| zbus::fdo::Error::Failed("serialization failed".to_owned()))?;
    if encoded.len() > MAX_REPLY_BYTES {
        return Err(zbus::fdo::Error::LimitsExceeded(
            "reply exceeds the 64 KiB API limit".to_owned(),
        ));
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_is_revisioned_bounded_and_uses_live_policy() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.set_output_ready();
        let (runtime, _) = ServiceRuntime::new(&policy, "missing".into());
        let value = serde_json::to_value(build_snapshot(&runtime)).expect("snapshot");
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["revision"], 1);
        assert_eq!(value["state"]["trust"], "trusted");
        assert_eq!(value["output"]["ready"], true);
        assert!(
            bounded_json(&build_snapshot(&runtime))
                .expect("bounded")
                .len()
                <= MAX_REPLY_BYTES
        );
    }
}
