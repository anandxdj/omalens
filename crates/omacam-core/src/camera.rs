//! Provider-neutral, acknowledged camera controls. Ranges are not mode promises.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCapabilities {
    pub cameras: Vec<Camera>,
    pub zoom: Option<ControlRange>,
    pub exposure_compensation: Option<ControlRange>,
    #[serde(default)]
    pub torch: bool,
    #[serde(default)]
    pub focus_modes: Vec<String>,
    #[serde(default)]
    pub screen_dim_supported: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub id: String,
    pub label: String,
    pub facing: String,
    #[serde(default)]
    pub modes: Vec<CameraMode>,
    #[serde(default)]
    pub frame_rates: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CameraMode {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ControlRange {
    pub min: f64,
    pub max: f64,
    #[serde(default, deserialize_with = "null_step_as_zero")]
    pub step: f64,
}

fn null_step_as_zero<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or_default())
}

impl ControlRange {
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        value.is_finite()
            && self.min.is_finite()
            && self.max.is_finite()
            && self.min <= value
            && value <= self.max
    }

    fn validate(&self) -> Result<(), &'static str> {
        if !self.min.is_finite()
            || !self.max.is_finite()
            || !self.step.is_finite()
            || self.min > self.max
            || self.step < 0.0
        {
            return Err("invalid control range");
        }
        Ok(())
    }
}

impl CameraCapabilities {
    /// Checks that values came from a plausible browser camera capability report.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate or malformed camera records, modes, or ranges.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.cameras.len() > 32 {
            return Err("too many cameras");
        }
        let mut ids = std::collections::HashSet::new();
        for camera in &self.cameras {
            if camera.id.is_empty()
                || camera.id.len() > 256
                || camera.label.len() > 256
                || camera.facing.len() > 32
                || !ids.insert(camera.id.as_str())
            {
                return Err("invalid camera description");
            }
            if camera.modes.len() > 256 || camera.frame_rates.len() > 64 {
                return Err("too many camera modes");
            }
            for mode in &camera.modes {
                if !(320..=4096).contains(&mode.width)
                    || !(240..=4096).contains(&mode.height)
                    || mode.width % 2 != 0
                    || mode.height % 2 != 0
                    || u64::from(mode.width) * u64::from(mode.height) > u64::from(3840_u32 * 2160)
                    || !mode.fps.is_finite()
                    || !(15.0..=60.0).contains(&mode.fps)
                {
                    return Err("invalid camera mode");
                }
            }
            if camera
                .frame_rates
                .iter()
                .any(|fps| !fps.is_finite() || !(1.0..=120.0).contains(fps))
            {
                return Err("invalid camera frame rate");
            }
        }
        if let Some(range) = &self.zoom {
            range.validate()?;
        }
        if let Some(range) = &self.exposure_compensation {
            range.validate()?;
        }
        if self.focus_modes.len() > 32
            || self
                .focus_modes
                .iter()
                .any(|mode| mode.is_empty() || mode.len() > 32)
        {
            return Err("invalid focus mode list");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppliedCameraState {
    pub camera_id: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub zoom: Option<f64>,
    pub exposure: Option<f64>,
    #[serde(default)]
    pub torch: bool,
    #[serde(default)]
    pub preview_mirrored: bool,
    #[serde(default)]
    pub screen_dimmed: bool,
}

impl AppliedCameraState {
    /// Validates a phone's report of settings that are actually applied now.
    ///
    /// # Errors
    ///
    /// Returns an error when the camera or any reported setting is unsupported.
    pub fn validate(&self, caps: &CameraCapabilities) -> Result<(), &'static str> {
        caps.validate()?;
        if !caps
            .cameras
            .iter()
            .any(|camera| camera.id == self.camera_id)
        {
            return Err("unknown applied camera");
        }
        if !(320..=4096).contains(&self.width)
            || !(240..=4096).contains(&self.height)
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || u64::from(self.width) * u64::from(self.height) > u64::from(3840_u32 * 2160)
            || !self.fps.is_finite()
            || !(15.0..=60.0).contains(&self.fps)
        {
            return Err("invalid applied format");
        }
        if self.zoom.is_some_and(|value| {
            !caps
                .zoom
                .as_ref()
                .is_some_and(|range| range.contains(value))
        }) {
            return Err("unsupported applied zoom");
        }
        if self.exposure.is_some_and(|value| {
            !caps
                .exposure_compensation
                .as_ref()
                .is_some_and(|range| range.contains(value))
        }) {
            return Err("unsupported applied exposure");
        }
        if self.torch && !caps.torch {
            return Err("unsupported applied torch");
        }
        if self.screen_dimmed && !caps.screen_dim_supported {
            return Err("unsupported applied screen dimming");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestedControls {
    pub camera_id: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub zoom: Option<f64>,
    pub exposure: Option<f64>,
    pub torch: Option<bool>,
    pub preview_mirrored: Option<bool>,
    pub screen_dimmed: Option<bool>,
    pub stop: Option<bool>,
}

impl RequestedControls {
    /// Checks a requested control set against the phone's reported capabilities.
    ///
    /// # Errors
    ///
    /// Returns an error when a requested camera mode or control is unsupported.
    pub fn validate(&self, caps: &CameraCapabilities) -> Result<(), &'static str> {
        caps.validate()?;
        if self
            .camera_id
            .as_ref()
            .is_some_and(|id| !caps.cameras.iter().any(|c| &c.id == id))
        {
            return Err("unknown camera");
        }
        if self
            .width
            .is_some_and(|v| !(320..=4096).contains(&v) || v % 2 != 0)
            || self
                .height
                .is_some_and(|v| !(240..=4096).contains(&v) || v % 2 != 0)
            || self
                .fps
                .is_some_and(|v| !v.is_finite() || !(15.0..=60.0).contains(&v))
        {
            return Err("unsupported format");
        }
        if let (Some(width), Some(height)) = (self.width, self.height)
            && u64::from(width) * u64::from(height) > u64::from(3840_u32 * 2160)
        {
            return Err("unsupported format");
        }
        if let (Some(width), Some(height), Some(fps)) = (self.width, self.height, self.fps) {
            let camera_id = self
                .camera_id
                .as_deref()
                .or_else(|| caps.cameras.first().map(|camera| camera.id.as_str()));
            if let Some(camera) =
                camera_id.and_then(|id| caps.cameras.iter().find(|camera| camera.id == id))
                && !camera.modes.is_empty()
                && !camera.modes.iter().any(|mode| {
                    mode.width == width && mode.height == height && (mode.fps - fps).abs() <= 0.5
                })
            {
                return Err("unsupported camera mode");
            }
        }
        if self
            .zoom
            .is_some_and(|v| !caps.zoom.as_ref().is_some_and(|r| r.contains(v)))
        {
            return Err("unsupported zoom");
        }
        if self.exposure.is_some_and(|v| {
            !caps
                .exposure_compensation
                .as_ref()
                .is_some_and(|r| r.contains(v))
        }) {
            return Err("unsupported exposure");
        }
        if self.torch.is_some() && !caps.torch {
            return Err("unsupported torch");
        }
        if self.screen_dimmed.is_some() && !caps.screen_dim_supported {
            return Err("unsupported screen dimming");
        }
        Ok(())
    }

    /// Reports whether an observed phone state reflects every requested field.
    /// Unrequested fields are deliberately ignored.
    #[must_use]
    pub fn is_reflected_by(&self, applied: &AppliedCameraState) -> bool {
        self.camera_id
            .as_ref()
            .is_none_or(|value| value == &applied.camera_id)
            && self.width.is_none_or(|value| value == applied.width)
            && self.height.is_none_or(|value| value == applied.height)
            && self
                .fps
                .is_none_or(|value| (value - applied.fps).abs() <= 0.5)
            && self.zoom.is_none_or(|value| {
                applied
                    .zoom
                    .is_some_and(|actual| (value - actual).abs() <= 0.001)
            })
            && self.exposure.is_none_or(|value| {
                applied
                    .exposure
                    .is_some_and(|actual| (value - actual).abs() <= 0.001)
            })
            && self.torch.is_none_or(|value| value == applied.torch)
            && self
                .preview_mirrored
                .is_none_or(|value| value == applied.preview_mirrored)
            && self
                .screen_dimmed
                .is_none_or(|value| value == applied.screen_dimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsupported_and_nonfinite_controls() {
        let caps = CameraCapabilities::default();
        assert!(
            RequestedControls {
                torch: Some(true),
                ..Default::default()
            }
            .validate(&caps)
            .is_err()
        );
        assert!(
            RequestedControls {
                fps: Some(f64::NAN),
                ..Default::default()
            }
            .validate(&caps)
            .is_err()
        );
        assert!(
            RequestedControls {
                stop: Some(true),
                ..Default::default()
            }
            .validate(&caps)
            .is_ok()
        );
    }

    #[test]
    fn accepts_unknown_capability_step_as_zero() {
        let range: ControlRange =
            serde_json::from_str(r#"{"min":1.0,"max":3.0,"step":null}"#).unwrap();
        assert!(range.step.abs() < f64::EPSILON);
        assert!(range.validate().is_ok());
    }

    #[test]
    fn only_accepts_applied_values_for_a_reported_camera_and_range() {
        let caps = CameraCapabilities {
            cameras: vec![Camera {
                id: "rear".into(),
                label: "Back camera".into(),
                facing: "environment".into(),
                modes: vec![CameraMode {
                    width: 1280,
                    height: 720,
                    fps: 30.0,
                }],
                frame_rates: vec![30.0],
            }],
            zoom: Some(ControlRange {
                min: 1.0,
                max: 3.0,
                step: 0.1,
            }),
            ..Default::default()
        };
        let applied = AppliedCameraState {
            camera_id: "rear".into(),
            width: 1280,
            height: 720,
            fps: 29.97,
            zoom: Some(1.5),
            ..Default::default()
        };
        assert!(applied.validate(&caps).is_ok());
        assert!(
            RequestedControls {
                camera_id: Some("rear".into()),
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                zoom: Some(1.5),
                ..Default::default()
            }
            .is_reflected_by(&applied)
        );
        assert!(
            RequestedControls {
                width: Some(1920),
                height: Some(1080),
                fps: Some(30.0),
                ..Default::default()
            }
            .validate(&caps)
            .is_err()
        );

        let mut forged = applied;
        forged.camera_id = "front".into();
        assert!(forged.validate(&caps).is_err());
    }
}
