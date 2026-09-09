use std::env;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

const USAGE: &str = "Usage:
  omacam-preview --probe
  omacam-preview --socket /owned/runtime/preview.sock [--wait-for-socket]";
const MAX_JPEG_BYTES: usize = 256 * 1024;
const READ_BUFFER_BYTES: usize = 16 * 1024;
const PREVIEW_CAPS: &str = "video/x-raw,format=RGBx,width=640,height=360,framerate=10/1";

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [flag] if flag == "--probe" => probe(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [flag, socket] if flag == "--socket" => run(Path::new(socket), false),
        [flag, socket, wait] if flag == "--socket" && wait == "--wait-for-socket" => {
            run(Path::new(socket), true)
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn probe() -> ExitCode {
    let missing = required_elements()
        .iter()
        .copied()
        .filter(|element| !gstreamer_has(element))
        .collect::<Vec<_>>();
    if missing.is_empty() && program_has("setpriv", "--help") {
        println!("preview consumer dependencies: READY");
        ExitCode::SUCCESS
    } else {
        let detail = if missing.is_empty() {
            "setpriv".to_owned()
        } else {
            missing.join(", ")
        };
        eprintln!("preview consumer dependencies: MISSING — {detail}");
        ExitCode::from(1)
    }
}

fn run(socket: &Path, wait_for_socket: bool) -> ExitCode {
    let socket = match wait_for_valid_socket(socket, wait_for_socket) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("invalid preview socket: {error}");
            return ExitCode::from(2);
        }
    };
    if !program_has("setpriv", "--help")
        || required_elements()
            .iter()
            .any(|element| !gstreamer_has(element))
    {
        eprintln!("required GStreamer elements are unavailable; run omacam-preview --probe");
        return ExitCode::from(1);
    }

    let mut child = match spawn_consumer(&socket) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("could not start preview consumer: {error}");
            return ExitCode::from(1);
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("preview consumer did not expose frame output");
        return ExitCode::from(1);
    };
    let result = forward_jpegs(stdout, io::stdout().lock());
    if result
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
    {
        let _ = child.kill();
        let _ = child.wait();
        return ExitCode::SUCCESS;
    }
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    match (result, status) {
        (Ok(()), Ok(status)) if status.success() => ExitCode::SUCCESS,
        (Err(error), _) => {
            eprintln!("preview frame stream failed: {error}");
            ExitCode::from(1)
        }
        (_, Ok(status)) => {
            eprintln!("preview pipeline exited with {status}");
            ExitCode::from(1)
        }
        (_, Err(error)) => {
            eprintln!("could not reap preview pipeline: {error}");
            ExitCode::from(1)
        }
    }
}

fn wait_for_valid_socket(socket: &Path, wait: bool) -> Result<PathBuf, String> {
    validate_socket_parent(socket)?;
    loop {
        match validate_socket(socket) {
            Ok(socket) => return Ok(socket),
            Err(error) if wait => match std::fs::symlink_metadata(socket) {
                Err(metadata_error) if metadata_error.kind() == io::ErrorKind::NotFound => {
                    thread::sleep(Duration::from_millis(250));
                }
                _ => return Err(error),
            },
            Err(error) => return Err(error),
        }
    }
}

fn spawn_consumer(socket: &Path) -> io::Result<Child> {
    build_consumer_command(socket).spawn()
}

fn build_consumer_command(socket: &Path) -> Command {
    let mut command = Command::new("setpriv");
    command.args([
        "--pdeathsig",
        "TERM",
        "gst-launch-1.0",
        "-q",
        "unixfdsrc",
        &format!("socket-path={}", socket.display()),
        "!",
        "queue",
        "max-size-buffers=1",
        "max-size-bytes=0",
        "max-size-time=0",
        "leaky=downstream",
        "!",
        "videorate",
        "drop-only=true",
        "!",
        PREVIEW_CAPS,
        "!",
        "jpegenc",
        "quality=70",
        "!",
        "fdsink",
        "fd=1",
        "sync=false",
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn forward_jpegs<R: Read, W: Write>(mut input: R, mut output: W) -> io::Result<()> {
    let mut parser = JpegParser::default();
    let mut chunk = [0_u8; READ_BUFFER_BYTES];
    loop {
        let count = input.read(&mut chunk)?;
        if count == 0 {
            return if parser.is_between_frames() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "preview ended inside a JPEG frame",
                ))
            };
        }
        for byte in &chunk[..count] {
            if let Some(frame) = parser.push(*byte)? {
                let encoded = STANDARD.encode(frame);
                output.write_all(encoded.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
    }
}

#[derive(Default)]
struct JpegParser {
    frame: Vec<u8>,
    previous_was_marker: bool,
}

impl JpegParser {
    fn is_between_frames(&self) -> bool {
        self.frame.is_empty()
    }

    fn push(&mut self, byte: u8) -> io::Result<Option<Vec<u8>>> {
        if self.frame.is_empty() {
            if self.previous_was_marker && byte == 0xd8 {
                self.frame.extend([0xff, 0xd8]);
                self.previous_was_marker = false;
            } else {
                self.previous_was_marker = byte == 0xff;
            }
            return Ok(None);
        }

        self.frame.push(byte);
        if self.frame.len() > MAX_JPEG_BYTES {
            self.frame.clear();
            self.previous_was_marker = false;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "preview JPEG exceeds the size limit",
            ));
        }
        let finished = self.previous_was_marker && byte == 0xd9;
        self.previous_was_marker = byte == 0xff;
        if finished {
            self.previous_was_marker = false;
            return Ok(Some(std::mem::take(&mut self.frame)));
        }
        Ok(None)
    }
}

fn validate_socket(socket: &Path) -> Result<PathBuf, String> {
    validate_socket_parent(socket)?;
    let metadata = std::fs::symlink_metadata(socket)
        .map_err(|error| format!("{}: {error}", socket.display()))?;
    if !metadata.file_type().is_socket() || metadata.file_type().is_symlink() {
        return Err("path must be an existing non-symlink Unix socket".to_owned());
    }
    Ok(socket.to_path_buf())
}

fn validate_socket_parent(socket: &Path) -> Result<(), String> {
    if !socket.is_absolute() {
        return Err("path must be absolute".to_owned());
    }
    let parent = socket
        .parent()
        .ok_or_else(|| "path has no parent directory".to_owned())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("{}: {error}", parent.display()))?;
    if !parent_metadata.file_type().is_dir()
        || parent_metadata.file_type().is_symlink()
        || parent_metadata.permissions().mode() & 0o077 != 0
    {
        return Err("socket parent must be a private non-symlink directory".to_owned());
    }
    Ok(())
}

fn required_elements() -> &'static [&'static str] {
    &["unixfdsrc", "queue", "videorate", "jpegenc", "fdsink"]
}

fn gstreamer_has(element: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn program_has(program: &str, argument: &str) -> bool {
    Command::new(program)
        .arg(argument)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_ignores_noise_and_frames_complete_jpegs() {
        let mut parser = JpegParser::default();
        let mut frames = Vec::new();
        for byte in [0, 0xff, 1, 0xff, 0xd8, 2, 3, 0xff, 0xd9, 9] {
            if let Some(frame) = parser.push(byte).unwrap() {
                frames.push(frame);
            }
        }

        assert_eq!(frames, vec![vec![0xff, 0xd8, 2, 3, 0xff, 0xd9]]);
        assert!(parser.is_between_frames());
    }

    #[test]
    fn parser_rejects_oversized_and_truncated_frames() {
        let mut oversized = vec![0xff, 0xd8];
        oversized.resize(MAX_JPEG_BYTES + 1, 0);
        assert!(forward_jpegs(oversized.as_slice(), Vec::new()).is_err());
        assert!(forward_jpegs(&[0xff, 0xd8, 1][..], Vec::new()).is_err());
    }

    #[test]
    fn complete_frames_become_independent_base64_lines() {
        let first = [0xff, 0xd8, 1, 0xff, 0xd9];
        let second = [0xff, 0xd8, 2, 0xff, 0xd9];
        let mut input = first.to_vec();
        input.extend(second);
        let mut output = Vec::new();

        forward_jpegs(input.as_slice(), &mut output).unwrap();

        let expected = format!("{}\n{}\n", STANDARD.encode(first), STANDARD.encode(second));
        assert_eq!(output, expected.as_bytes());
    }

    #[test]
    fn consumer_graph_has_no_h264_decoder_and_is_freshness_bounded() {
        let command = build_consumer_command(Path::new("/run/user/1000/omacam/preview.sock"));
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>();

        assert!(!args.iter().any(|arg| arg.contains("264")));
        assert!(args.iter().any(|arg| arg.as_ref() == "max-size-buffers=1"));
        assert!(args.iter().any(|arg| arg.as_ref() == "leaky=downstream"));
        assert!(args.iter().any(|arg| arg.as_ref() == PREVIEW_CAPS));
    }
}
