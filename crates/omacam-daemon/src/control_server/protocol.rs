//! Strict newline-delimited messages shared by control and capture transports.
use serde::{Deserialize, Serialize};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::time::timeout;

use crate::{CLIENT_STEP_TIMEOUT, MAX_CONTROL_MESSAGE_BYTES};

pub(crate) async fn read_json_line<T: serde::de::DeserializeOwned, R: AsyncRead + Unpin>(
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
    CameraControl {
        command_id: String,
        generation: u64,
        controls: omacam_core::camera::RequestedControls,
    },
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
    CameraState {
        command_id: String,
        generation: u64,
        capabilities: omacam_core::camera::CameraCapabilities,
        applied: omacam_core::camera::AppliedCameraState,
        error: Option<String>,
    },
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
            "camera_state"
                if exact(&[
                    "type",
                    "command_id",
                    "generation",
                    "capabilities",
                    "applied",
                    "error",
                ]) =>
            {
                let command_id = object["command_id"]
                    .as_str()
                    .filter(|id| id.len() <= 128)
                    .ok_or("invalid camera command id")?
                    .to_owned();
                let generation = object["generation"]
                    .as_u64()
                    .ok_or("invalid camera generation")?;
                let capabilities = serde_json::from_value(object["capabilities"].clone())
                    .map_err(|_| "invalid camera capabilities")?;
                let applied = serde_json::from_value(object["applied"].clone())
                    .map_err(|_| "invalid applied camera state")?;
                let error: Option<String> = serde_json::from_value(object["error"].clone())
                    .map_err(|_| "invalid camera error")?;
                if error.as_ref().is_some_and(|value| value.len() > 256) {
                    return Err("camera error too long");
                }
                Ok(Self::CameraState {
                    command_id,
                    generation,
                    capabilities,
                    applied,
                    error,
                })
            }
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

pub(crate) async fn write_json_line<T: Serialize, W: AsyncWrite + Unpin>(
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
    fn camera_state_accepts_the_phone_capability_and_applied_state_shapes() {
        let message = serde_json::json!({
            "type": "camera_state",
            "command_id": "zoom-1",
            "generation": 9,
            "capabilities": {
                "cameras": [{
                    "id": "0",
                    "label": "Rear camera",
                    "facing": "environment",
                    "modes": [{"width": 1280, "height": 720, "fps": 30.0}],
                    "frameRates": [30.0]
                }],
                "zoom": {"min": 1.0, "max": 4.0, "step": 0.01},
                "torch": true,
                "focusModes": [],
                "screenDimSupported": true
            },
            "applied": {
                "cameraId": "0",
                "width": 1280,
                "height": 720,
                "fps": 30.0,
                "zoom": 1.5,
                "exposure": null,
                "torch": false,
                "previewMirrored": false,
                "screenDimmed": true
            },
            "error": null
        });

        let command: ControlCommand = serde_json::from_value(message).expect("valid camera state");
        assert!(matches!(
            command,
            ControlCommand::CameraState { command_id, generation: 9, applied, .. }
                if command_id == "zoom-1" && applied.camera_id == "0" && applied.zoom == Some(1.5)
        ));
    }

    #[test]
    fn camera_control_reply_matches_the_shared_partial_control_contract() {
        let reply = ControlReply::CameraControl {
            command_id: "select-front".into(),
            generation: 9,
            controls: omacam_core::camera::RequestedControls {
                camera_id: Some("1".into()),
                ..Default::default()
            },
        };
        let value = serde_json::to_value(reply).expect("serialize camera control");

        assert_eq!(value["type"], "camera_control");
        assert_eq!(value["command_id"], "select-front");
        assert_eq!(value["controls"]["cameraId"], "1");
    }
}
