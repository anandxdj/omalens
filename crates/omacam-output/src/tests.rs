use crate::cli::{extract_backend_option, parse_media_binding, split_preview_option};
use crate::config::*;
use crate::mux::*;
use crate::pipeline::*;
use crate::preview::*;
use crate::service::*;
use omacam_core::media::{
    ControlConnectionId, MediaBinding, MediaRecord, MediaRecordKind, MediaSessionId,
    MediaStreamValidator, OutputControlCommand, PeerIdentity, write_output_control, write_record,
};
use std::env;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const I420_FRAME_BYTES: usize = OUTPUT_WIDTH * OUTPUT_HEIGHT * 3 / 2;

const BINDING: MediaBinding = MediaBinding {
    peer_identity: PeerIdentity::new([1; 32]),
    control_connection_id: ControlConnectionId::new([2; 16]),
    media_session_id: MediaSessionId::new([3; 16]),
    generation: 9,
};

#[test]
fn backend_defaults_to_v4l2_and_can_be_selected_by_cli_or_environment() {
    assert_eq!(OutputBackend::default(), OutputBackend::V4l2);

    let args = vec![
        "--device".to_owned(),
        "/dev/video42".to_owned(),
        "--service-stdin".to_owned(),
        "--backend".to_owned(),
        "pipewire".to_owned(),
    ];
    let (filtered, backend) = extract_backend_option(&args, "v4l2").unwrap();
    assert_eq!(filtered, ["--device", "/dev/video42", "--service-stdin"]);
    assert_eq!(backend, OutputBackend::PipeWire);

    let (filtered, backend) =
        extract_backend_option(&["--service-stdin".to_owned()], "pipewire").unwrap();
    assert_eq!(filtered, ["--service-stdin"]);
    assert_eq!(backend, OutputBackend::PipeWire);
    let (_, backend) =
        extract_backend_option(&["--backend".to_owned(), "v4l2".to_owned()], "pipewire").unwrap();
    assert_eq!(backend, OutputBackend::V4l2);
    assert!(extract_backend_option(&["--backend".to_owned()], "v4l2").is_err());
    assert!(
        extract_backend_option(
            &[
                "--backend".to_owned(),
                "v4l2".to_owned(),
                "--backend".to_owned(),
                "pipewire".to_owned()
            ],
            "v4l2"
        )
        .is_err()
    );
}

#[test]
fn adaptive_video_format_is_bounded_and_computes_exact_i420_size() {
    let args = [
        "--width".to_owned(),
        "3840".to_owned(),
        "--height".to_owned(),
        "2160".to_owned(),
        "--fps".to_owned(),
        "60".to_owned(),
    ];
    let format = VideoFormat::parse(&args).unwrap();
    assert_eq!(format.frame_bytes(), 12_441_600);
    let oversized = [
        "--width".to_owned(),
        "4096".to_owned(),
        "--height".to_owned(),
        "2160".to_owned(),
        "--fps".to_owned(),
        "60".to_owned(),
    ];
    assert!(VideoFormat::parse(&oversized).is_err());
}

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
    assert!(
        args.iter()
            .any(|arg| arg.as_ref() == "video/x-h264,stream-format=byte-stream")
    );
    assert!(!args.iter().any(|arg| {
        arg.contains("profile=constrained-baseline") || arg.contains("video/x-h264,width=")
    }));
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

#[test]
fn service_keeps_v4l2_pipeline_separate_from_the_single_decoder() {
    let raw = build_raw_pipeline_command(
        Path::new("/dev/video42"),
        Some(Path::new("/run/user/1000/omacam/preview.sock")),
        VideoFormat::default(),
    );
    let raw_args = raw
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        raw_args
            .iter()
            .filter(|argument| argument.as_str() == "openh264dec")
            .count(),
        0
    );
    assert_eq!(
        raw_args
            .iter()
            .filter(|argument| argument.as_str() == "v4l2sink")
            .count(),
        1
    );
    assert!(raw_args.iter().any(|argument| argument == "rawvideoparse"));
    assert!(
        raw_args
            .iter()
            .any(|argument| argument == "wait-for-connection=false")
    );

    let decoder = build_decoder_command(VideoFormat::default());
    let decoder_args = decoder
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        decoder_args
            .iter()
            .filter(|argument| argument.as_str() == "openh264dec")
            .count(),
        1
    );
    assert_eq!(
        decoder_args
            .iter()
            .filter(|argument| argument.as_str() == "v4l2sink")
            .count(),
        0
    );
}

#[test]
fn pipewire_pipeline_provides_a_video_source_without_touching_v4l2() {
    let raw = build_raw_pipeline_command_for_backend(
        Path::new(""),
        Some(Path::new("/run/user/1000/omacam/preview.sock")),
        VideoFormat::default(),
        OutputBackend::PipeWire,
    );
    let raw_args = raw
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        raw_args
            .iter()
            .filter(|argument| argument.as_str() == "pipewiresink")
            .count(),
        1
    );
    assert_eq!(
        raw_args
            .iter()
            .filter(|argument| argument.as_str() == "v4l2sink")
            .count(),
        0
    );
    assert!(raw_args.iter().any(|argument| argument == "mode=provide"));
    assert!(
        raw_args
            .iter()
            .any(|argument| argument.contains("media.class=Video/Source"))
    );
    assert!(
        raw_args
            .iter()
            .any(|argument| argument == "client-name=OmaCam")
    );
    assert!(raw_args.iter().any(|argument| argument == "rawvideoparse"));
    assert!(
        raw_args
            .iter()
            .any(|argument| argument == "wait-for-connection=false")
    );
    assert!(
        !raw_args
            .iter()
            .any(|argument| argument.starts_with("device="))
    );
    assert_eq!(
        raw_args
            .iter()
            .filter(|argument| argument.as_str() == "openh264dec")
            .count(),
        0
    );
}

#[test]
fn raw_frame_queue_is_bounded_and_freshness_biased() {
    let queue = RawFrameQueue::new(I420_FRAME_BYTES);
    queue.push(vec![1; I420_FRAME_BYTES]);
    queue.push(vec![2; I420_FRAME_BYTES]);
    queue.push(vec![3; I420_FRAME_BYTES]);

    let latest = queue.take_latest().expect("latest frame");
    assert_eq!(latest[0], 3);
    assert!(queue.take_latest().is_none());
}

#[test]
fn neutral_frame_is_fixed_i420_black_without_stale_bytes() {
    let frame = black_i420_frame(VideoFormat::default());
    assert_eq!(frame.len(), I420_FRAME_BYTES);
    assert!(
        frame[..OUTPUT_WIDTH * OUTPUT_HEIGHT]
            .iter()
            .all(|byte| *byte == 16)
    );
    assert!(
        frame[OUTPUT_WIDTH * OUTPUT_HEIGHT..]
            .iter()
            .all(|byte| *byte == 128)
    );
}

#[test]
fn service_reader_bounds_media_before_payload_allocation() {
    let record = MediaRecord {
        kind: MediaRecordKind::H264AccessUnit,
        key_frame: true,
        sequence: 0,
        presentation_time_us: 1_000,
        payload: vec![0, 0, 0, 1, 0x65],
    };
    let mut bytes = Vec::new();
    write_record(&mut bytes, BINDING, &record).unwrap();
    bytes[100..104].copy_from_slice(
        &(u32::try_from(omacam_core::media::MAX_H264_ACCESS_UNIT_BYTES).unwrap() + 1).to_be_bytes(),
    );
    let error = read_service_input(&mut Cursor::new(bytes)).unwrap_err();
    assert!(error.contains("1 MiB"));
}

#[test]
fn service_reader_preserves_control_and_media_order() {
    let frame = MediaRecord {
        kind: MediaRecordKind::H264AccessUnit,
        key_frame: true,
        sequence: 0,
        presentation_time_us: 1_000,
        payload: vec![0, 0, 0, 1, 0x65],
    };
    let mut bytes = Vec::new();
    write_output_control(&mut bytes, OutputControlCommand::Bind(BINDING)).unwrap();
    write_record(&mut bytes, BINDING, &frame).unwrap();
    let mut reader = Cursor::new(bytes);
    assert!(matches!(
        read_service_input(&mut reader).unwrap(),
        Some(ServiceInput::Control(OutputControlCommand::Bind(binding))) if binding == BINDING
    ));
    assert!(matches!(
        read_service_input(&mut reader).unwrap(),
        Some(ServiceInput::Media { payload, .. }) if payload == frame.payload
    ));
}

#[test]
fn raw_mux_reset_and_rebind_keep_one_neutral_writer_alive() {
    let mut sink = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("cat sink");
    let input = sink.stdin.take().expect("sink stdin");
    let mut mux = RawMux::spawn(input, VideoFormat::default());

    mux.activate().expect("first bind");
    mux.frames.push(vec![7; I420_FRAME_BYTES]);
    std::thread::sleep(Duration::from_millis(50));
    mux.deactivate().expect("reset");
    mux.frames.push(vec![9; I420_FRAME_BYTES]);
    mux.activate().expect("rebind");
    mux.clear();
    mux.deactivate().expect("second reset");
    assert!(mux.check_health().is_ok());
    mux.shutdown().expect("mux shutdown");
    let _ = sink.kill();
    let _ = sink.wait();
}
