use std::env;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};

use omacam_core::media::{
    ControlConnectionId, MediaBinding, MediaRecordKind, MediaSessionId, MediaStreamValidator,
    PeerIdentity,
};

const USAGE: &str = "Usage:
  omacam-output --probe
  omacam-output --device /dev/videoN
  omacam-output --device /dev/videoN --framed-h264-stdin --peer <64_HEX> --connection <32_HEX> --session <32_HEX> --generation <U64>";
// Leave scheduling margin below the externally observable 500 ms privacy bound.
const STALE_FRAME_LIMIT_NS: &str = "400000000";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--probe" => probe(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [flag, device] if flag == "--device" => run_writer(Path::new(device), None),
        [
            flag,
            device,
            media,
            peer_flag,
            peer,
            connection_flag,
            connection,
            session_flag,
            session,
            generation_flag,
            generation,
        ] if flag == "--device"
            && media == "--framed-h264-stdin"
            && peer_flag == "--peer"
            && connection_flag == "--connection"
            && session_flag == "--session"
            && generation_flag == "--generation" =>
        {
            match parse_media_binding(peer, connection, session, generation) {
                Ok(binding) => run_writer(Path::new(device), Some(binding)),
                Err(error) => {
                    eprintln!("invalid media binding: {error}");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn probe() -> ExitCode {
    if required_elements(false)
        .iter()
        .all(|element| gstreamer_has(element))
    {
        println!("neutral output dependencies: READY");
        if required_elements(true)
            .iter()
            .all(|element| gstreamer_has(element))
        {
            println!("H.264 live-input dependencies: READY");
        } else {
            println!("H.264 live-input dependencies: MISSING");
        }
        ExitCode::SUCCESS
    } else {
        eprintln!("neutral output dependencies: MISSING (need videotestsrc and v4l2sink)");
        ExitCode::from(1)
    }
}

fn parse_media_binding(
    peer: &str,
    connection: &str,
    session: &str,
    generation: &str,
) -> Result<MediaBinding, String> {
    let peer_identity = PeerIdentity::from_hex(peer).map_err(|error| error.to_string())?;
    let control_connection_id =
        ControlConnectionId::from_hex(connection).map_err(|error| error.to_string())?;
    let media_session_id = MediaSessionId::from_hex(session).map_err(|error| error.to_string())?;
    let generation = generation
        .parse::<u64>()
        .map_err(|_| "generation must be an unsigned 64-bit integer".to_owned())?;
    Ok(MediaBinding {
        peer_identity,
        control_connection_id,
        media_session_id,
        generation,
    })
}

fn run_writer(device: &Path, media: Option<MediaBinding>) -> ExitCode {
    let canonical_device = match validate_video_device(device) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid output device: {error}");
            return ExitCode::from(2);
        }
    };

    if !required_elements(media.is_some())
        .iter()
        .all(|element| gstreamer_has(element))
    {
        eprintln!("required GStreamer elements are unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }

    let mut child = match spawn_pipeline(&canonical_device, media.is_some()) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("could not start GStreamer: {error}");
            return ExitCode::from(1);
        }
    };

    if let Some(binding) = media {
        let Some(decoder_input) = child.stdin.take() else {
            eprintln!("GStreamer media input was not available");
            let _ = child.kill();
            let _ = child.wait();
            return ExitCode::from(1);
        };
        let mut validator = MediaStreamValidator::new(binding);
        let mut input = io::stdin().lock();
        if let Err(error) = forward_media(&mut input, decoder_input, &mut validator) {
            eprintln!("live media rejected; output is returning to neutral: {error}");
        }
    }

    let status = child.wait();

    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("output pipeline exited with {status}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("could not start GStreamer: {error}");
            ExitCode::from(1)
        }
    }
}

fn spawn_pipeline(device: &Path, framed_h264: bool) -> io::Result<Child> {
    let device_arg = format!("device={}", device.display());
    let mut command = Command::new("gst-launch-1.0");
    command.arg("-q");
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
            "videoconvert",
            "!",
            "video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1",
            "!",
            "v4l2sink",
        ]);
        command.arg(device_arg).arg("sync=true").args([
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
            "video/x-h264,stream-format=byte-stream",
            "!",
            "h264parse",
            "disable-passthrough=true",
            "config-interval=-1",
            "!",
            // MediaCodec does not promise an SPS timing VUI, so parsed H.264
            // may legitimately advertise an unknown input rate. Dimensions
            // and profile stay constrained before decode; videorate below
            // establishes the fixed 30 fps output clock.
            "video/x-h264,stream-format=byte-stream,alignment=au,profile=constrained-baseline,width=1280,height=720",
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
        command
            .args([
                "videotestsrc",
                "pattern=black",
                "is-live=true",
                "!",
                "video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1",
                "!",
                "v4l2sink",
            ])
            .arg(device_arg)
            .arg("sync=true")
            .stdin(Stdio::null());
    }
    command.spawn()
}

fn forward_media<R: io::Read, W: io::Write>(
    reader: &mut R,
    mut decoder_input: W,
    validator: &mut MediaStreamValidator,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(record) = validator.read_next(reader)? {
        match record.kind {
            MediaRecordKind::H264AccessUnit => {
                decoder_input.write_all(&record.payload)?;
                decoder_input.flush()?;
            }
            MediaRecordKind::Stop => return Ok(()),
        }
    }
    Ok(())
}

fn required_elements(h264_stdin: bool) -> &'static [&'static str] {
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

fn gstreamer_has(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn validate_video_device(device: &Path) -> Result<PathBuf, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use omacam_core::media::{ControlConnectionId, MediaRecord, PeerIdentity, write_record};
    use std::io::Cursor;

    const BINDING: MediaBinding = MediaBinding {
        peer_identity: PeerIdentity::new([1; 32]),
        control_connection_id: ControlConnectionId::new([2; 16]),
        media_session_id: MediaSessionId::new([3; 16]),
        generation: 9,
    };

    #[test]
    fn ordinary_files_are_never_accepted_as_video_devices() {
        let path = env::temp_dir().join(format!("omacam-output-test-{}", std::process::id()));
        std::fs::write(&path, b"not a device").unwrap();

        let result = validate_video_device(&path);

        std::fs::remove_file(path).unwrap();
        assert_eq!(result, Err("path must resolve to /dev/videoN".to_owned()));
    }

    #[test]
    fn nonexistent_paths_are_rejected() {
        let result = validate_video_device(Path::new("/dev/video-does-not-exist"));

        assert!(result.unwrap_err().contains("No such file"));
    }

    #[test]
    fn framed_ingress_forwards_only_valid_access_units_before_stop() {
        let frame = MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: true,
            sequence: 0,
            presentation_time_us: 1_000,
            payload: vec![0, 0, 0, 1, 0x65],
        };
        let stop = MediaRecord {
            kind: MediaRecordKind::Stop,
            key_frame: false,
            sequence: 1,
            presentation_time_us: 0,
            payload: Vec::new(),
        };
        let late = MediaRecord {
            sequence: 2,
            presentation_time_us: 1_033,
            ..frame.clone()
        };
        let mut input = Vec::new();
        write_record(&mut input, BINDING, &frame).unwrap();
        write_record(&mut input, BINDING, &stop).unwrap();
        write_record(&mut input, BINDING, &late).unwrap();
        let mut output = Vec::new();
        let mut validator = MediaStreamValidator::new(BINDING);

        forward_media(&mut Cursor::new(input), &mut output, &mut validator).unwrap();

        assert_eq!(output, frame.payload);
        assert!(validator.is_invalidated());
    }

    #[test]
    fn framed_ingress_rejects_stale_generation_without_decoder_bytes() {
        let stale_binding = MediaBinding {
            generation: BINDING.generation - 1,
            ..BINDING
        };
        let frame = MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: true,
            sequence: 0,
            presentation_time_us: 1_000,
            payload: vec![0, 0, 0, 1, 0x65],
        };
        let mut input = Vec::new();
        write_record(&mut input, stale_binding, &frame).unwrap();
        let mut output = Vec::new();
        let mut validator = MediaStreamValidator::new(BINDING);

        assert!(forward_media(&mut Cursor::new(input), &mut output, &mut validator).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn cli_binding_requires_every_scope_identifier() {
        let binding =
            parse_media_binding(&"01".repeat(32), &"02".repeat(16), &"03".repeat(16), "9").unwrap();

        assert_eq!(binding, BINDING);
        assert!(parse_media_binding("short", &"02".repeat(16), &"03".repeat(16), "9").is_err());
    }
}
