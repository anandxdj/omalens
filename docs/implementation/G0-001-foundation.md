# G0-001 — Desktop foundation and synthetic output

Status: implemented; host spike passed; broader G0 evidence pending.

Requirements: R10, R11, R18, R19 foundations. Gate: G0, with reusable work toward G2.

## Objective and result

Establish a minimal compilable desktop project, encode the core capture safety states, inspect host readiness without mutation, prove a synthetic 720p30 V4L2 producer/consumer path, and create a native read-only Omarchy panel artifact.

Changed paths: root Cargo workspace and README; `crates/omacam-core`; `crates/omacam-daemon`; `crates/omacam-output`; `omarchy-plugin`; G0 documentation.

## Verification performed

- `cargo test --workspace`: 14 tests passed (10 core, 2 diagnostics, 2 output validation).
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `qmllint -I /usr/share/omarchy/shell omarchy-plugin/BarWidget.qml omarchy-plugin/Panel.qml`: passed.
- `git diff --check`: passed.
- Output probe found GStreamer `videotestsrc`, `v4l2sink`, and `webrtcbin` after prerequisite installation.
- Temporary `v4l2loopback` device `/dev/video42`, label `OmaCam Camera`, `exclusive_caps=1`: producer negotiated YUYV 1280×720 at 30 fps.
- `v4l2-ctl` consumed three complete frames totaling 5,529,600 bytes.
- Read-only diagnostics reported the output writer `READY` while running and `INACTIVE` after stop.

Environment details and limitations are recorded in [G0-environment.md](G0-environment.md).

## Safety and cleanup

No network, USB configuration, ADB authorization, firewall, VPN, desktop configuration, audio routing, or existing video device was changed. The output CLI accepts only an explicit canonical `/dev/video*` character device and invokes GStreamer with an argument array. The temporary writer was stopped; its module/device and raw test capture were removed. Rust, GStreamer plugins, v4l2loopback DKMS, matching headers, Android platform tools, and their declared Arch dependencies remain installed for development. The first ADB query started the normal per-user ADB server; no device connected to it.

## Pending work

- The QML artifact is linted but has not been loaded into the live shell.
- Chrome, Firefox, OBS, sandboxed/portal consumers, stable ownership/provisioning, crash persistence, and resource budgets remain untested.
- A OnePlus Nord 4 was observed on USB but is not ADB-authorized. No Android SDK, Camera2 capture, authenticated media, pairing, or preview path exists yet.
- Public naming, license, repository URL, Android application ID/signing, and final service identifiers remain open.

Next task: G0-002 should prove the read-only panel in the actual Omarchy shell and select a bounded preview mechanism. G0-003 should establish an Android test device/toolchain and evaluate one maintained authenticated media path before G1 contracts are frozen.
