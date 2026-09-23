use std::time::Duration;

// Leave scheduling margin below the externally observable 500 ms privacy bound.
pub(crate) const STALE_FRAME_LIMIT_NS: &str = "400000000";
pub(crate) const OUTPUT_WIDTH: usize = 1280;
pub(crate) const OUTPUT_HEIGHT: usize = 720;
pub(crate) const OUTPUT_FPS: usize = 30;
pub(crate) const RAW_FRAME_QUEUE_CAPACITY: usize = 2;
pub(crate) const STALE_FRAME_LIMIT: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OutputBackend {
    #[default]
    V4l2,
    PipeWire,
}

impl OutputBackend {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "v4l2" => Ok(Self::V4l2),
            "pipewire" => Ok(Self::PipeWire),
            _ => Err("output backend must be v4l2 or pipewire".to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VideoFormat {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) fps: usize,
}

impl Default for VideoFormat {
    fn default() -> Self {
        Self {
            width: OUTPUT_WIDTH,
            height: OUTPUT_HEIGHT,
            fps: OUTPUT_FPS,
        }
    }
}

impl VideoFormat {
    pub(crate) fn parse(values: &[String]) -> Result<Self, String> {
        if values.is_empty() {
            return Ok(Self::default());
        }
        let [width_flag, width, height_flag, height, fps_flag, fps] = values else {
            return Err("adaptive format requires --width N --height N --fps N".to_owned());
        };
        if width_flag != "--width" || height_flag != "--height" || fps_flag != "--fps" {
            return Err("adaptive format requires --width N --height N --fps N".to_owned());
        }
        let format = Self {
            width: width.parse().map_err(|_| "width must be an integer")?,
            height: height.parse().map_err(|_| "height must be an integer")?,
            fps: fps.parse().map_err(|_| "fps must be an integer")?,
        };
        let pixels = format.width.saturating_mul(format.height);
        if !(320..=4096).contains(&format.width)
            || !(240..=4096).contains(&format.height)
            || !format.width.is_multiple_of(2)
            || !format.height.is_multiple_of(2)
            || pixels > 3840 * 2160
            || !(15..=60).contains(&format.fps)
        {
            return Err(
                "format must be even-sized, 320..4096 by 240..4096, at most 4K, and 15..60 fps"
                    .to_owned(),
            );
        }
        Ok(format)
    }

    pub(crate) fn frame_bytes(self) -> usize {
        self.width * self.height * 3 / 2
    }

    pub(crate) fn frame_interval(self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.fps as u64)
    }
}
