use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::config::{OutputBackend, STALE_FRAME_LIMIT_NS, VideoFormat};
use crate::preview;

pub(crate) fn spawn_pipeline(
    device: &Path,
    framed_h264: bool,
    preview_socket: Option<&Path>,
) -> io::Result<Child> {
    build_pipeline_command(device, framed_h264, preview_socket).spawn()
}

pub(crate) fn spawn_raw_pipeline(
    device: &Path,
    preview_socket: Option<&Path>,
    format: VideoFormat,
    backend: OutputBackend,
) -> io::Result<Child> {
    build_raw_pipeline_command_for_backend(device, preview_socket, format, backend).spawn()
}

pub(crate) fn build_decoder_command(format: VideoFormat) -> Command {
    let mut command = Command::new("setpriv");
    command.args(["--pdeathsig", "TERM", "gst-launch-1.0"]);
    command.args([
        "fdsrc",
        "fd=0",
        "do-timestamp=true",
        "!",
        "h264parse",
        "disable-passthrough=true",
        "config-interval=-1",
        "!",
        "openh264dec",
        "discard-corrupted-frames=true",
        "max-errors=8",
        "!",
        "videoconvert",
        "!",
        "videoscale",
        "!",
        &format!(
            "video/x-raw,format=I420,width={},height={}",
            format.width, format.height
        ),
        "!",
        "fdsink",
        "fd=1",
        "sync=false",
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    command
}

#[cfg(test)]
pub(crate) fn build_raw_pipeline_command(
    device: &Path,
    preview_socket: Option<&Path>,
    format: VideoFormat,
) -> Command {
    build_raw_pipeline_command_for_backend(device, preview_socket, format, OutputBackend::V4l2)
}

pub(crate) fn build_raw_pipeline_command_for_backend(
    device: &Path,
    preview_socket: Option<&Path>,
    format: VideoFormat,
    backend: OutputBackend,
) -> Command {
    let device_arg = format!("device={}", device.display());
    let mut command = Command::new("setpriv");
    command.args(["--pdeathsig", "TERM", "gst-launch-1.0", "-q"]);
    command.args([
        "fdsrc",
        "fd=0",
        &format!("blocksize={}", format.frame_bytes()),
        "do-timestamp=true",
        "!",
        "rawvideoparse",
        "format=i420",
        &format!("width={}", format.width),
        &format!("height={}", format.height),
        &format!("framerate={}/1", format.fps),
        "!",
        "tee",
        "name=decoded",
    ]);
    command.args([
        "decoded.",
        "!",
        "queue",
        "max-size-buffers=2",
        "max-size-bytes=0",
        "max-size-time=0",
        "leaky=downstream",
        "!",
        "videoconvert",
        "!",
        &format!(
            "video/x-raw,format=YUY2,width={},height={},framerate={}/1",
            format.width, format.height, format.fps
        ),
        "!",
    ]);
    match backend {
        OutputBackend::V4l2 => {
            command.args(["v4l2sink", &device_arg, "sync=false"]);
        }
        OutputBackend::PipeWire => {
            command.args([
                "pipewiresink",
                "mode=provide",
                "client-name=OmaCam",
                "stream-properties=props,media.class=Video/Source,media.role=Camera,node.name=omacam.camera,node.description=\"OmaCam Camera\",node.virtual=true",
                "sync=false",
                "async=false",
                "enable-last-sample=false",
            ]);
        }
    }
    preview::add_preview_branch(&mut command, preview_socket);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

pub(crate) fn build_pipeline_command(
    device: &Path,
    framed_h264: bool,
    preview_socket: Option<&Path>,
) -> Command {
    let device_arg = format!("device={}", device.display());
    let mut command = Command::new("setpriv");
    command.args(["--pdeathsig", "TERM", "gst-launch-1.0", "-q"]);
    if framed_h264 {
        command.args([
            "compositor",
            "name=mix",
            "background=black",
            "force-live=true",
            "ignore-inactive-pads=true",
            "sink_0::zorder=0",
            "sink_1::zorder=1",
            "sink_1::repeat-after-eos=false",
            &format!("sink_1::max-last-buffer-repeat={STALE_FRAME_LIMIT_NS}"),
            "!",
            "tee",
            "name=decoded",
        ]);
        add_output_branches(&mut command, &device_arg, preview_socket);
        command.args([
            "videotestsrc",
            "pattern=black",
            "is-live=true",
            "!",
            "video/x-raw,format=I420,width=1280,height=720,framerate=30/1",
            "!",
            "mix.sink_0",
            "fdsrc",
            "fd=0",
            "do-timestamp=true",
            "!",
            // Encoders may omit timing VUI; leave profile and dimensions
            // unconstrained until decode, then normalize the output format.
            "video/x-h264,stream-format=byte-stream",
            "!",
            "h264parse",
            "disable-passthrough=true",
            "config-interval=-1",
            "!",
            "openh264dec",
            "discard-corrupted-frames=true",
            "max-errors=8",
            "!",
            "queue",
            "max-size-buffers=2",
            "max-size-bytes=0",
            "max-size-time=0",
            "leaky=downstream",
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "!",
            "videorate",
            "drop-only=true",
            "!",
            "video/x-raw,format=I420,width=1280,height=720,framerate=30/1",
            "!",
            "mix.sink_1",
        ]);
        command.stdin(Stdio::piped());
    } else {
        command.args([
            "videotestsrc",
            "pattern=black",
            "is-live=true",
            "!",
            "video/x-raw,format=I420,width=1280,height=720,framerate=30/1",
            "!",
            "tee",
            "name=decoded",
        ]);
        add_output_branches(&mut command, &device_arg, preview_socket);
        command.stdin(Stdio::null());
    }
    command
}

fn add_output_branches(command: &mut Command, device_arg: &str, preview_socket: Option<&Path>) {
    command.args([
        "decoded.",
        "!",
        "queue",
        "max-size-buffers=2",
        "max-size-bytes=0",
        "max-size-time=0",
        "leaky=downstream",
        "!",
        "videoconvert",
        "!",
        "video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1",
        "!",
        "v4l2sink",
        device_arg,
        "sync=true",
    ]);
    preview::add_preview_branch(command, preview_socket);
}
pub(crate) fn required_elements(h264_stdin: bool) -> &'static [&'static str] {
    if h264_stdin {
        &[
            "videotestsrc",
            "compositor",
            "fdsrc",
            "h264parse",
            "openh264dec",
            "queue",
            "videoconvert",
            "videoscale",
            "videorate",
            "v4l2sink",
        ]
    } else {
        &["videotestsrc", "v4l2sink"]
    }
}

pub(crate) fn service_elements() -> &'static [&'static str] {
    &[
        "fdsrc",
        "rawvideoparse",
        "tee",
        "queue",
        "videoconvert",
        "videoscale",
        "h264parse",
        "openh264dec",
        "fdsink",
    ]
}

pub(crate) fn service_sink_element(backend: OutputBackend) -> &'static str {
    match backend {
        OutputBackend::V4l2 => "v4l2sink",
        OutputBackend::PipeWire => "pipewiresink",
    }
}

pub(crate) fn gstreamer_has(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn command_has(program: &str, argument: &str) -> bool {
    Command::new(program)
        .arg(argument)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn validate_video_device(device: &Path) -> Result<PathBuf, String> {
    let canonical = device
        .canonicalize()
        .map_err(|error| format!("{}: {error}", device.display()))?;
    let parent = canonical.parent();
    let name = canonical.file_name().and_then(|name| name.to_str());
    if parent != Some(Path::new("/dev")) || !name.is_some_and(|name| name.starts_with("video")) {
        return Err("path must resolve to /dev/videoN".to_owned());
    }

    let metadata = canonical
        .metadata()
        .map_err(|error| format!("{}: {error}", canonical.display()))?;
    if !metadata.file_type().is_char_device() {
        return Err("path is not a character device".to_owned());
    }

    Ok(canonical)
}
