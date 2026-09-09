use std::env;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};

use omacam_core::media::{
    ControlConnectionId, MEDIA_HEADER_BYTES, MEDIA_MAGIC, MediaBinding, MediaRecordKind,
    MediaSessionId, MediaStreamValidator, OUTPUT_CONTROL_HEADER_BYTES, OUTPUT_CONTROL_MAGIC,
    OutputControlCommand, PeerIdentity, parse_output_control,
};

const USAGE: &str = "Usage:
  omacam-output --probe
  omacam-output --device /dev/videoN [--preview-socket /owned/runtime/preview.sock]
  omacam-output --device /dev/videoN --service-stdin [--preview-socket /owned/runtime/preview.sock]
  omacam-output --device /dev/videoN --framed-h264-stdin --peer <64_HEX> --connection <32_HEX> --session <32_HEX> --generation <U64> [--preview-socket /owned/runtime/preview.sock]";
// Leave scheduling margin below the externally observable 500 ms privacy bound.
const STALE_FRAME_LIMIT_NS: &str = "400000000";
const PREVIEW_CAPS: &str = "video/x-raw,format=RGBx,width=640,height=360,framerate=15/1";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (args, preview_socket) = split_preview_option(&args);
    match args {
        [flag] if flag == "--probe" => probe(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [flag, device] if flag == "--device" => run_writer(Path::new(device), None, preview_socket),
        [flag, device, service] if flag == "--device" && service == "--service-stdin" => {
            run_service_writer(Path::new(device), preview_socket)
        }
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
                Ok(binding) => run_writer(Path::new(device), Some(binding), preview_socket),
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

fn split_preview_option(args: &[String]) -> (&[String], Option<&Path>) {
    if args.len() >= 2 && args[args.len() - 2] == "--preview-socket" {
        (
            &args[..args.len() - 2],
            Some(Path::new(&args[args.len() - 1])),
        )
    } else {
        (args, None)
    }
}

fn probe() -> ExitCode {
    if command_has("setpriv", "--help")
        && required_elements(false)
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
        if preview_elements()
            .iter()
            .all(|element| gstreamer_has(element))
        {
            println!("shared preview dependencies: READY");
        } else {
            println!("shared preview dependencies: MISSING");
        }
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "neutral output dependencies: MISSING (need setpriv, videotestsrc, and v4l2sink)"
        );
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

fn run_writer(
    device: &Path,
    media: Option<MediaBinding>,
    preview_socket: Option<&Path>,
) -> ExitCode {
    let canonical_device = match validate_video_device(device) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid output device: {error}");
            return ExitCode::from(2);
        }
    };

    let preview_socket = match preview_socket.map(validate_preview_socket).transpose() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid preview socket: {error}");
            return ExitCode::from(2);
        }
    };

    if !command_has("setpriv", "--help")
        || !required_elements(media.is_some())
            .iter()
            .all(|element| gstreamer_has(element))
        || preview_socket.is_some()
            && !preview_elements()
                .iter()
                .all(|element| gstreamer_has(element))
    {
        eprintln!("required GStreamer elements are unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }

    let mut child = match spawn_pipeline(
        &canonical_device,
        media.is_some(),
        preview_socket.as_deref(),
    ) {
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

fn run_service_writer(device: &Path, preview_socket: Option<&Path>) -> ExitCode {
    let canonical_device = match validate_video_device(device) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid output device: {error}");
            return ExitCode::from(2);
        }
    };
    let preview_socket = match preview_socket.map(validate_preview_socket).transpose() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid preview socket: {error}");
            return ExitCode::from(2);
        }
    };
    if !command_has("setpriv", "--help")
        || !required_elements(true)
            .iter()
            .all(|element| gstreamer_has(element))
        || preview_socket.is_some()
            && !preview_elements()
                .iter()
                .all(|element| gstreamer_has(element))
    {
        eprintln!("required GStreamer elements are unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }

    let mut child = match spawn_pipeline(&canonical_device, true, preview_socket.as_deref()) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("could not start GStreamer: {error}");
            return ExitCode::from(1);
        }
    };
    let Some(mut decoder_input) = child.stdin.take() else {
        eprintln!("GStreamer media input was not available");
        let _ = child.kill();
        let _ = child.wait();
        return ExitCode::from(1);
    };

    let result = forward_service(&mut io::stdin().lock(), &mut decoder_input);
    drop(decoder_input);
    if let Err(error) = &result {
        eprintln!("output service protocol failed: {error}");
        let _ = child.kill();
    }
    match child.wait() {
        Ok(status) if status.success() && result.is_ok() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("output pipeline exited with {status}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("could not reap GStreamer: {error}");
            ExitCode::from(1)
        }
    }
}

/// Reads the service control envelope and media records from one private pipe.
///
/// A Bind/Reset only changes the validator; it never recreates the `GStreamer`
/// pipeline. A reset therefore leaves the virtual-camera handle attached while
/// the compositor's neutral branch takes over. Every media header is validated
/// against the currently bound generation before its payload is allocated.
fn forward_service<R: io::Read, W: io::Write>(
    reader: &mut R,
    decoder_input: &mut W,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut active_binding = None;
    let mut validator: Option<MediaStreamValidator> = None;
    loop {
        let Some(prefix) = read_prefix(reader)? else {
            return Ok(());
        };
        if prefix == MEDIA_MAGIC {
            let mut header = [0_u8; MEDIA_HEADER_BYTES];
            header[..MEDIA_MAGIC.len()].copy_from_slice(&prefix);
            reader.read_exact(&mut header[MEDIA_MAGIC.len()..])?;
            let Some(current) = validator.as_mut() else {
                return Err("media arrived before an output binding".into());
            };
            let pending = current.validate_header(&header)?;
            let mut payload = vec![0_u8; pending.payload_length()];
            reader.read_exact(&mut payload)?;
            let record = current.finish_record(pending, payload)?;
            if record.kind == MediaRecordKind::Stop {
                active_binding = None;
                validator = None;
            } else {
                decoder_input.write_all(&record.payload)?;
                decoder_input.flush()?;
            }
            continue;
        }
        if prefix != OUTPUT_CONTROL_MAGIC {
            return Err("output service command magic is invalid".into());
        }
        let mut header = [0_u8; OUTPUT_CONTROL_HEADER_BYTES];
        header[..OUTPUT_CONTROL_MAGIC.len()].copy_from_slice(&prefix);
        reader.read_exact(&mut header[OUTPUT_CONTROL_MAGIC.len()..])?;
        match parse_output_control(&header)? {
            OutputControlCommand::Bind(binding) => {
                // A new bind is valid only after the previous owner has been
                // reset by the daemon. The child also rejects a reset that is
                // not for the currently active binding below.
                if active_binding.is_some() {
                    return Err("output bind requires a reset of the active generation".into());
                }
                active_binding = Some(binding);
                validator = Some(MediaStreamValidator::new(binding));
            }
            OutputControlCommand::Reset(binding) => {
                if active_binding.is_some_and(|current| current != binding) {
                    return Err("output reset belongs to a different generation".into());
                }
                active_binding = None;
                validator = None;
            }
            OutputControlCommand::Shutdown => return Ok(()),
        }
    }
}

fn read_prefix<R: io::Read>(reader: &mut R) -> io::Result<Option<[u8; 8]>> {
    let mut prefix = [0_u8; 8];
    match reader.read(&mut prefix[..1])? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!("one-byte read returned more than one byte"),
    }
    reader.read_exact(&mut prefix[1..])?;
    Ok(Some(prefix))
}

fn spawn_pipeline(
    device: &Path,
    framed_h264: bool,
    preview_socket: Option<&Path>,
) -> io::Result<Child> {
    build_pipeline_command(device, framed_h264, preview_socket).spawn()
}

fn build_pipeline_command(
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
    if let Some(socket) = preview_socket {
        command.args([
            "decoded.",
            "!",
            "queue",
            "max-size-buffers=1",
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
            PREVIEW_CAPS,
            "!",
            "unixfdsink",
            &format!("socket-path={}", socket.display()),
            "wait-for-connection=false",
            "sync=false",
        ]);
    }
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

fn preview_elements() -> &'static [&'static str] {
    &[
        "tee",
        "queue",
        "videoconvert",
        "videoscale",
        "videorate",
        "unixfdsink",
    ]
}

fn gstreamer_has(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_has(program: &str, argument: &str) -> bool {
    Command::new(program)
        .arg(argument)
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

fn validate_preview_socket(socket: &Path) -> Result<PathBuf, String> {
    if !socket.is_absolute() {
        return Err("path must be absolute".to_owned());
    }
    if socket.exists() {
        return Err("path already exists; refusing to replace it".to_owned());
    }
    let parent = socket
        .parent()
        .ok_or_else(|| "path has no parent directory".to_owned())?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("{}: {error}", parent.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("parent must be a non-symlink directory".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "parent directory must not be accessible by group or other users".to_owned(),
            );
        }
    }
    Ok(socket.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacam_core::media::{
        ControlConnectionId, MediaRecord, OutputControlCommand, PeerIdentity, write_output_control,
        write_record,
    };
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

    #[test]
    fn preview_option_is_terminal_and_explicit() {
        let args = vec![
            "--device".to_owned(),
            "/dev/video42".to_owned(),
            "--preview-socket".to_owned(),
            "/run/user/1000/omacam/preview.sock".to_owned(),
        ];
        let (base, preview) = split_preview_option(&args);

        assert_eq!(base, &args[..2]);
        assert_eq!(
            preview,
            Some(Path::new("/run/user/1000/omacam/preview.sock"))
        );
        let misplaced = vec![
            "--preview-socket".to_owned(),
            "/tmp/preview.sock".to_owned(),
            "--device".to_owned(),
            "/dev/video42".to_owned(),
        ];
        assert_eq!(
            split_preview_option(&misplaced),
            (misplaced.as_slice(), None)
        );
    }

    #[test]
    fn preview_socket_never_replaces_existing_paths() {
        let parent = env::temp_dir().join(format!("omacam-preview-test-{}", std::process::id()));
        std::fs::create_dir(&parent).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let socket = parent.join("preview.sock");
        assert_eq!(validate_preview_socket(&socket).unwrap(), socket);
        std::fs::write(&socket, b"owned by somebody else").unwrap();
        assert!(validate_preview_socket(&socket).is_err());
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn shared_preview_branches_after_exactly_one_h264_decoder() {
        let command = build_pipeline_command(
            Path::new("/dev/video42"),
            true,
            Some(Path::new("/run/user/1000/omacam/preview.sock")),
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>();

        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_ref() == "openh264dec")
                .count(),
            1
        );
        assert_eq!(args.iter().filter(|arg| arg.as_ref() == "tee").count(), 1);
        assert!(args.iter().any(|arg| arg.as_ref() == PREVIEW_CAPS));
        assert!(
            args.iter()
                .any(|arg| { arg.as_ref() == "socket-path=/run/user/1000/omacam/preview.sock" })
        );
        assert!(
            args.iter()
                .any(|arg| arg.as_ref() == "wait-for-connection=false")
        );
    }

    #[test]
    fn service_input_rebinds_without_forwarding_stale_generation_or_control_bytes() {
        let next_binding = MediaBinding {
            generation: BINDING.generation + 1,
            ..BINDING
        };
        let first = MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: true,
            sequence: 0,
            presentation_time_us: 1_000,
            payload: vec![0, 0, 0, 1, 0x65],
        };
        let second = MediaRecord {
            sequence: 0,
            presentation_time_us: 2_000,
            ..first.clone()
        };
        let stale = MediaRecord {
            sequence: 1,
            presentation_time_us: 1_033,
            ..first.clone()
        };
        let mut input = Vec::new();
        write_output_control(&mut input, OutputControlCommand::Bind(BINDING)).unwrap();
        write_record(&mut input, BINDING, &first).unwrap();
        write_output_control(&mut input, OutputControlCommand::Reset(BINDING)).unwrap();
        write_output_control(&mut input, OutputControlCommand::Bind(next_binding)).unwrap();
        // This old record must not cross the rebind boundary.
        write_record(&mut input, BINDING, &stale).unwrap();

        let mut output = Vec::new();
        assert!(forward_service(&mut Cursor::new(input), &mut output).is_err());
        assert_eq!(output, first.payload);

        let mut fresh_input = Vec::new();
        write_output_control(&mut fresh_input, OutputControlCommand::Bind(next_binding)).unwrap();
        write_record(&mut fresh_input, next_binding, &second).unwrap();
        write_output_control(&mut fresh_input, OutputControlCommand::Shutdown).unwrap();
        let mut fresh_output = Vec::new();
        forward_service(&mut Cursor::new(fresh_input), &mut fresh_output).unwrap();
        assert_eq!(fresh_output, second.payload);
    }

    #[test]
    fn service_input_reset_drops_frames_until_a_new_bind() {
        let frame = MediaRecord {
            kind: MediaRecordKind::H264AccessUnit,
            key_frame: true,
            sequence: 0,
            presentation_time_us: 1_000,
            payload: vec![0, 0, 0, 1, 0x65],
        };
        let mut input = Vec::new();
        write_output_control(&mut input, OutputControlCommand::Bind(BINDING)).unwrap();
        write_output_control(&mut input, OutputControlCommand::Reset(BINDING)).unwrap();
        write_output_control(&mut input, OutputControlCommand::Shutdown).unwrap();

        let mut output = Vec::new();
        forward_service(&mut Cursor::new(input), &mut output).unwrap();
        assert!(output.is_empty());

        // The same generation cannot be accepted after Reset without a fresh
        // Bind command; a late frame is therefore rejected before forwarding.
        let mut late = Vec::new();
        write_record(&mut late, BINDING, &frame).unwrap();
        assert!(forward_service(&mut Cursor::new(late), &mut Vec::new()).is_err());
    }

    #[test]
    fn service_input_rejects_rebind_without_reset() {
        let next_binding = MediaBinding {
            generation: BINDING.generation + 1,
            ..BINDING
        };
        let mut input = Vec::new();
        write_output_control(&mut input, OutputControlCommand::Bind(BINDING)).unwrap();
        write_output_control(&mut input, OutputControlCommand::Bind(next_binding)).unwrap();

        let error = forward_service(&mut Cursor::new(input), &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("requires a reset"));
    }
}
