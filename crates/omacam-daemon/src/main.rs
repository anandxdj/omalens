use std::env;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use omacam_core::SessionPolicy;

const USAGE: &str = "Usage: omacam-daemon <doctor|snapshot>";

fn main() {
    let exit_code = match env::args().nth(1).as_deref() {
        Some("doctor") => {
            print!("{}", doctor_report());
            0
        }
        Some("snapshot") => {
            println!("{:?}", SessionPolicy::default().snapshot());
            0
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            0
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };

    std::process::exit(exit_code);
}

fn doctor_report() -> String {
    let mut report = String::from("OmaCam host readiness (read-only)\n");
    add_command_check(&mut report, "Omarchy", "omarchy", &["version"]);
    add_command_check(&mut report, "Quickshell", "quickshell", &["--version"]);
    add_command_check(&mut report, "GStreamer", "gst-launch-1.0", &["--version"]);
    add_gstreamer_element(&mut report, "GStreamer V4L2 output", "v4l2sink");
    add_gstreamer_element(&mut report, "GStreamer WebRTC", "webrtcbin");
    add_command_check(&mut report, "V4L2 tools", "v4l2-ctl", &["--version"]);
    add_command_check(&mut report, "ADB (future adapter)", "adb", &["version"]);

    let video_devices = count_video_devices();
    let _ = writeln!(
        report,
        "{:<28} {} ({video_devices} device(s))",
        "Linux video devices",
        if video_devices == 0 {
            "MISSING"
        } else {
            "READY"
        }
    );
    let module_loaded = Path::new("/sys/module/v4l2loopback").exists();
    let _ = writeln!(
        report,
        "{:<28} {}",
        "v4l2loopback module",
        if module_loaded { "READY" } else { "MISSING" }
    );
    let oma_device = find_omacam_labeled_device();
    let _ = writeln!(
        report,
        "{:<28} {}",
        "OmaCam-labeled device",
        oma_device
            .as_deref()
            .map_or("MISSING".to_owned(), |path| format!(
                "FOUND — {}",
                path.display()
            ))
    );
    let writer_ready = oma_device.as_deref().is_some_and(output_writer_ready);
    let _ = writeln!(
        report,
        "{:<28} {}",
        "OmaCam output writer",
        if writer_ready { "READY" } else { "INACTIVE" }
    );
    report.push_str(
        "\nMISSING means setup or qualification is still required; no repair was attempted.\n",
    );
    report
}

fn add_command_check(report: &mut String, label: &str, program: &str, args: &[&str]) {
    let result = Command::new(program).args(args).output();
    match result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let version = stdout
                .lines()
                .chain(stderr.lines())
                .find(|line| !line.trim().is_empty())
                .unwrap_or("available");
            let _ = writeln!(report, "{label:<28} READY — {}", version.trim());
        }
        Ok(output) => {
            let _ = writeln!(report, "{label:<28} ERROR — exited with {}", output.status);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = writeln!(report, "{label:<28} MISSING — {program}");
        }
        Err(error) => {
            let _ = writeln!(report, "{label:<28} ERROR — {error}");
        }
    }
}

fn add_gstreamer_element(report: &mut String, label: &str, element: &str) {
    let status = Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .filter(std::process::ExitStatus::success);
    let _ = writeln!(
        report,
        "{label:<28} {} — {element}",
        if status.is_some() { "READY" } else { "MISSING" }
    );
}

fn count_video_devices() -> usize {
    std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("video"))
        .count()
}

fn find_omacam_labeled_device() -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/video4linux")
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let name = std::fs::read_to_string(entry.path().join("name")).ok()?;
            if name.trim() == "OmaCam Camera" {
                Some(Path::new("/dev").join(entry.file_name()))
            } else {
                None
            }
        })
}

fn output_writer_ready(device: &Path) -> bool {
    let output = Command::new("v4l2-ctl")
        .arg("-d")
        .arg(device)
        .arg("--all")
        .output();
    output.is_ok_and(|output| {
        output.status.success() && reports_capture_only(&String::from_utf8_lossy(&output.stdout))
    })
}

fn reports_capture_only(report: &str) -> bool {
    report.contains("Video Capture") && !report.contains("Video Output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_report_explains_that_it_does_not_repair() {
        let report = doctor_report();

        assert!(report.starts_with("OmaCam host readiness (read-only)"));
        assert!(report.contains("no repair was attempted"));
        assert!(report.contains("v4l2loopback module"));
        assert!(report.contains("OmaCam output writer"));
    }

    #[test]
    fn capture_only_caps_mean_a_writer_is_attached() {
        assert!(reports_capture_only(
            "Capabilities\n  Video Capture\n  Streaming"
        ));
        assert!(!reports_capture_only(
            "Capabilities\n  Video Capture\n  Video Output\n  Streaming"
        ));
    }
}
