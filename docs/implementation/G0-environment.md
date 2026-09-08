# G0 environment record

Status: in progress. Inspected 2026-09-08 with read-only commands unless noted.

## Host baseline

| Item | Observed |
| --- | --- |
| Architecture | x86-64 |
| Kernel | Linux 7.1.9-arch1-2 |
| Omarchy | 4.0.2-1 |
| Quickshell | 0.3.1-1 |
| Qt | 6.11.2-2 |
| GStreamer | 1.28.6-2 |
| PipeWire | 1.6.8-1 |
| WirePlumber | 0.5.15-1 |
| Camera portal packages | xdg-desktop-portal 1.22.1-2; xdg-desktop-portal-hyprland 1.4.1-1 |
| GPU | Intel Iris Xe plus NVIDIA RTX 3050 Mobile |
| Rust | 1.98.0, installed during implementation with user authorization |

## Readiness findings

- The inspected Omarchy shell supports manifest schema 1 and on-demand bar-widget plugins. The plugin installer cannot install privileged dependencies.
- GStreamer 1.28.6 output and WebRTC plugins were installed during implementation. `v4l2sink` and `webrtcbin` now probe successfully.
- `v4l2loopback-dkms` 0.15.4-2 is installed for the matching kernel. Before the spike, no loopback module or sandbox-visible `/dev/video*` device was observed.
- Android platform tools 37.0.0-3 were installed during implementation; Gradle and the Android SDK are absent. Java 26.0.2.1 is installed.
- Secure Boot state could not be read because `mokutil` is absent. Treat it as unknown.
- Read-only USB enumeration outside the sandbox identified an OPPO/OnePlus Nord 4 (`22d9:276a`). `adb devices -l` returned no authorized devices, so its OS/build/camera capabilities are still unknown. OmaCam did not enable debugging or change its USB mode.
- The user session D-Bus could not be queried from the sandbox, so PipeWire service state remains unverified even though packages and the GStreamer PipeWire plugin are present.

## Implemented foundation

- Dependency-free Rust session-policy crate with independent trust, connection, capture, and output states.
- Explicit consent, session generation invalidation, ten-second capture lease, and 500 ms stale-frame neutralization rules.
- Read-only host diagnostics command.
- Synthetic neutral-output worker that requires an explicit existing `/dev/videoN` character device and uses an argument-array GStreamer process. It does not provision or select a device.
- A temporary, non-persistent `/dev/video42` labeled `OmaCam Camera` was created with `exclusive_caps=1`. The worker negotiated YUYV 1280×720 at 30 fps. `v4l2-ctl` consumed three frames totaling 5,529,600 bytes, exactly three 1,843,200-byte frames. This proves the producer/consumer format path on this host; Chrome, Firefox, OBS, persistence, ownership, and idle-resource claims remain untested.
- Diagnostics distinguished the running writer (`READY`) from the stopped writer (`INACTIVE`). The temporary writer was stopped, the test module/device was removed, and the raw `/tmp` capture was deleted after verification. Installed packages remain as development prerequisites.

## Required evidence before G0 exit

- Design and qualify persistent provisioning and ownership for the already-proven V4L2 output path without disturbing other devices.
- Select and benchmark maintained authenticated media/codec dependencies.
- Prove preview delivery in the actual Omarchy shell.
- Connect at least one Android phone and record its model, OS, camera behavior, and lifecycle results.
- Measure the defined resource and latency targets. Inspect USB-tether and hotspot continuity on authorized test setups.
- Replace placeholder package/service identifiers only after the public name and repository ownership are settled.
