//! Read-only host and provider diagnostics.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use crate::{default_data_path, load_trust_record};

pub(crate) fn doctor_report() -> String {
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

pub(crate) fn provider_report() -> String {
    let mut report = String::from("OmaCam provider preflight (read-only)\n");
    let uvc_candidates = existing_uvc_candidates();
    if uvc_candidates.is_empty() {
        report
            .push_str("1. Native UVC       UNAVAILABLE — no existing UVC video source detected\n");
    } else {
        let _ = writeln!(
            report,
            "1. Native UVC       REVIEW — {} existing source(s); select only after confirming the phone",
            uvc_candidates.len()
        );
        for candidate in uvc_candidates {
            let _ = writeln!(report, "   {candidate}");
        }
    }
    report.push_str(
        "2. Zero-install web UNQUALIFIED — secure local browser bootstrap is not implemented\n",
    );
    let trust_path = default_data_path("trusted-phone.json");
    match load_trust_record(&trust_path) {
        Ok(record) => {
            let _ = writeln!(
                report,
                "3. Companion       READY — trusted {} (fallback)",
                record.phone_name
            );
        }
        Err(_) => report.push_str("3. Companion       NEEDS PAIRING — fallback is not ready\n"),
    }
    report.push_str(
        "No USB function, route, DNS, firewall, VPN, or interface change was attempted.\n",
    );
    report
}

fn existing_uvc_candidates() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let device_name = entry.file_name().to_string_lossy().into_owned();
            let device = Path::new("/dev").join(&device_name);
            let output = Command::new("udevadm")
                .args(["info", "--query=property", "--name"])
                .arg(&device)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let properties = String::from_utf8_lossy(&output.stdout);
            describe_uvc_candidate(&device_name, &properties)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
}

pub(crate) fn describe_uvc_candidate(device_name: &str, properties: &str) -> Option<String> {
    let property = |key: &str| {
        properties
            .lines()
            .find_map(|line| line.strip_prefix(key).map(str::to_owned))
    };
    let driver = property("ID_USB_DRIVER=").or_else(|| property("ID_USB_INTERFACES="));
    if !driver
        .as_deref()
        .is_some_and(|value| value.contains("uvcvideo") || value.contains(":0e"))
    {
        return None;
    }
    let label = property("ID_V4L_PRODUCT=")
        .or_else(|| property("ID_MODEL_FROM_DATABASE="))
        .or_else(|| property("ID_MODEL="))
        .unwrap_or_else(|| "unnamed UVC source".to_owned());
    let vendor = property("ID_VENDOR_FROM_DATABASE=")
        .or_else(|| property("ID_VENDOR="))
        .unwrap_or_else(|| "unknown vendor".to_owned());
    Some(format!("/dev/{device_name}: {vendor} — {label}"))
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

pub(crate) fn omacam_output_ready() -> bool {
    find_omacam_labeled_device()
        .as_deref()
        .is_some_and(output_writer_ready)
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

pub(crate) fn reports_capture_only(report: &str) -> bool {
    report.contains("Video Capture") && !report.contains("Video Output")
}
