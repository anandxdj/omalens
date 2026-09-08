use std::env;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "Usage:\n  omacam-output --probe\n  omacam-output --device /dev/videoN";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--probe" => probe(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [flag, device] if flag == "--device" => run_writer(Path::new(device)),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn probe() -> ExitCode {
    if gstreamer_has("videotestsrc") && gstreamer_has("v4l2sink") {
        println!("synthetic output dependencies: READY");
        ExitCode::SUCCESS
    } else {
        eprintln!("synthetic output dependencies: MISSING (need videotestsrc and v4l2sink)");
        ExitCode::from(1)
    }
}

fn run_writer(device: &Path) -> ExitCode {
    let canonical_device = match validate_video_device(device) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("invalid output device: {error}");
            return ExitCode::from(2);
        }
    };

    if !gstreamer_has("videotestsrc") || !gstreamer_has("v4l2sink") {
        eprintln!("required GStreamer elements are unavailable; run omacam-output --probe");
        return ExitCode::from(1);
    }

    let device_arg = format!("device={}", canonical_device.display());
    let status = Command::new("gst-launch-1.0")
        .args([
            "-q",
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
        .stdin(Stdio::null())
        .status();

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
}
