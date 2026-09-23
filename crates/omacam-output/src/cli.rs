use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use omacam_core::media::{
    ControlConnectionId, MediaBinding, MediaSessionId, MediaStreamValidator, PeerIdentity,
};

use crate::config::{OutputBackend, VideoFormat};
use crate::pipeline::{
    command_has, gstreamer_has, required_elements, service_elements, service_sink_element,
    spawn_pipeline, validate_video_device,
};
use crate::preview::{elements as preview_elements, validate_preview_socket};
use crate::service::{ServiceOutput, forward_media};

const USAGE: &str = "Usage:
  omacam-output [--backend v4l2|pipewire] --probe
  omacam-output --device /dev/videoN [--preview-socket /owned/runtime/preview.sock]
  omacam-output --device /dev/videoN --service-stdin [--width N --height N --fps N] [--preview-socket /owned/runtime/preview.sock]
  omacam-output --backend pipewire --service-stdin [--width N --height N --fps N] [--preview-socket /owned/runtime/preview.sock]
  omacam-output --device /dev/videoN --framed-h264-stdin --peer <64_HEX> --connection <32_HEX> --session <32_HEX> --generation <U64> [--preview-socket /owned/runtime/preview.sock]
  OMACAM_OUTPUT_BACKEND=v4l2|pipewire selects the service backend; --backend takes precedence.";

pub(crate) fn run() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (args, backend) = match extract_backend_option(
        &args,
        &env::var("OMACAM_OUTPUT_BACKEND").unwrap_or_else(|_| "v4l2".to_owned()),
    ) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let (args, preview_socket) = split_preview_option(&args);
    match args {
        [flag] if flag == "--probe" => probe(backend),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [flag, _device] if flag == "--device" && backend == OutputBackend::PipeWire => {
            eprintln!("PipeWire output is available with --service-stdin");
            ExitCode::from(2)
        }
        [flag, device] if flag == "--device" => run_writer(Path::new(device), None, preview_socket),
        [flag, device, service, format @ ..]
            if flag == "--device" && service == "--service-stdin" =>
        {
            match VideoFormat::parse(format) {
                Ok(format) => {
                    run_service_writer(Path::new(device), preview_socket, format, backend)
                }
                Err(error) => {
                    eprintln!("invalid output format: {error}");
                    ExitCode::from(2)
                }
            }
        }
        [service, format @ ..]
            if service == "--service-stdin" && backend == OutputBackend::PipeWire =>
        {
            match VideoFormat::parse(format) {
                Ok(format) => run_service_writer(Path::new(""), preview_socket, format, backend),
                Err(error) => {
                    eprintln!("invalid output format: {error}");
                    ExitCode::from(2)
                }
            }
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
            if backend == OutputBackend::PipeWire {
                eprintln!("PipeWire output is available with --service-stdin");
                return ExitCode::from(2);
            }
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

pub(crate) fn extract_backend_option(
    args: &[String],
    environment_backend: &str,
) -> Result<(Vec<String>, OutputBackend), String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut selected = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--backend" {
            if selected.is_some() {
                return Err("--backend may only be specified once".to_owned());
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--backend requires v4l2 or pipewire".to_owned())?;
            selected = Some(OutputBackend::parse(value)?);
            index += 2;
        } else {
            filtered.push(args[index].clone());
            index += 1;
        }
    }
    let backend = selected.unwrap_or(OutputBackend::parse(environment_backend)?);
    Ok((filtered, backend))
}
pub(crate) fn split_preview_option(args: &[String]) -> (&[String], Option<&Path>) {
    if args.len() >= 2 && args[args.len() - 2] == "--preview-socket" {
        (
            &args[..args.len() - 2],
            Some(Path::new(&args[args.len() - 1])),
        )
    } else {
        (args, None)
    }
}

fn probe(backend: OutputBackend) -> ExitCode {
    if !command_has("setpriv", "--help") {
        eprintln!("setpriv is unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }
    if backend == OutputBackend::V4l2 {
        let neutral_ready = required_elements(false)
            .iter()
            .all(|element| gstreamer_has(element));
        println!(
            "neutral output dependencies: {}",
            if neutral_ready { "READY" } else { "MISSING" }
        );
        let h264_ready = required_elements(true)
            .iter()
            .all(|element| gstreamer_has(element));
        println!(
            "H.264 live-input dependencies: {}",
            if h264_ready { "READY" } else { "MISSING" }
        );
    }
    let service_ready = service_elements()
        .iter()
        .all(|element| gstreamer_has(element))
        && gstreamer_has(service_sink_element(backend));
    println!(
        "service-scoped {backend:?} output dependencies: {}",
        if service_ready { "READY" } else { "MISSING" }
    );
    let preview_ready = preview_elements()
        .iter()
        .all(|element| gstreamer_has(element));
    println!(
        "shared preview dependencies: {}",
        if preview_ready { "READY" } else { "MISSING" }
    );
    if service_ready {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

pub(crate) fn parse_media_binding(
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

fn run_service_writer(
    device: &Path,
    preview_socket: Option<&Path>,
    format: VideoFormat,
    backend: OutputBackend,
) -> ExitCode {
    let canonical_device = match if backend == OutputBackend::V4l2 {
        validate_video_device(device)
    } else {
        Ok(PathBuf::new())
    } {
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
        || !service_elements()
            .iter()
            .all(|element| gstreamer_has(element))
        || !gstreamer_has(service_sink_element(backend))
        || preview_socket.is_some()
            && !preview_elements()
                .iter()
                .all(|element| gstreamer_has(element))
    {
        eprintln!("required GStreamer elements are unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }

    let mut service = match ServiceOutput::spawn(
        &canonical_device,
        preview_socket.as_deref(),
        format,
        backend,
    ) {
        Ok(service) => service,
        Err(error) => {
            eprintln!("could not start service-scoped output: {error}");
            return ExitCode::from(1);
        }
    };

    let result = service.run(io::stdin());
    if let Err(error) = &result {
        eprintln!("output service protocol failed: {error}");
    }
    let shutdown = service.shutdown();
    match (result, shutdown) {
        (Ok(()), Ok(())) => ExitCode::SUCCESS,
        (Err(error), _) => {
            eprintln!("output service stopped: {error}");
            ExitCode::from(1)
        }
        (Ok(()), Err(error)) => {
            eprintln!("output service shutdown failed: {error}");
            ExitCode::from(1)
        }
    }
}
