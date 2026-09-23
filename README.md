# OmaCam

OmaCam is an Omarchy-first, local-only phone-to-desktop camera bridge. It has
two providers: a native Android companion using Camera2, hardware H.264, and an
authenticated TLS connection on port 47123; and an experimental, zero-install
phone-browser provider using local HTTPS signaling and host-only H.264 WebRTC on
port 8443. The latter uses no STUN, TURN, cloud relay, or public signaling
service. “Local-only” describes the intended routing, not a guarantee against
unusual VPN or network configurations.

The native path previously streamed recognizable 720p30 video from a OnePlus
Nord 4 to an OmaCam-owned `/dev/video42`. That is a historical physical check,
not proof that the current uncommitted changes have been retested on a phone.
**This worktree is an implementation candidate, not a qualified release.**

The desktop output has a shared H.264 decoder with a V4L2-loopback backend and
an experimental PipeWire Video/Source backend. Both feed the same bounded local
preview branch. PipeWire consumer compatibility has not been verified yet.

Read [the product documentation](docs/README.md) before contributing. The
[current implementation and qualification plan](docs/implementation/2026-09-23-browser-control-and-modularity.md)
records the current scope and required physical checks.

## Current implementation (2026-09-23)

- Browser provider: phone capture page and desktop control room, capability and
  applied-state messages, camera controls, shared decoded preview, local dim
  screen with wake-lock lifecycle, authorized signaling, and Stop/failure paths.
  Camera and format options are constrained by what a browser reports. The
  server rejects live resolution/FPS changes because output format is fixed at
  session start; the UI still needs a coherent pre-start selection/restart flow.
- Native provider: Camera2 capability discovery and acknowledged control
  messages are implemented in the companion/service code. The native camera is
  still a 720p30 path. Its D-Bus control API is **not yet connected to the new
  desktop browser control room**; do not present the two providers as one
  finished UI.
- Output: modular CLI, configuration, decoder, pipeline, preview, and service
  modules. V4L2 remains the default. Set `OMACAM_OUTPUT_BACKEND=pipewire` for
  the experimental PipeWire service backend; this is not yet a validated
  default across applications.
- Verification on this machine: the Rust workspace suite and browser Node
  lifecycle tests passed during this handoff. There is no connected handset or
  local Android SDK, so Android builds, optical latency, thermal behavior, and
  consumer compatibility are unverified.

The source boundaries are `crates/omacam-core` (shared contracts),
`crates/omacam-daemon/src/browser/` (browser HTTP/session/RTC/preview),
`crates/omacam-daemon/src/control_server/` and `capture/` (native service),
`crates/omacam-output/src/` (one decoder and selectable sinks), `webapp/`
(phone/desktop browser modules), and `android/app/` (native companion). The
modular organization is inspired by the separate controller/service/adapter
responsibilities in [OpenAssets](https://github.com/anandxdj/open_assets),
without copying its TypeScript stack.

## Current developer commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --test webapp/tests/*.test.js
cargo run -p omacam-daemon -- doctor
cargo run -p omacam-daemon -- pair status
cargo run -p omacam-daemon -- service --listen <LAN_IP:PORT> --output-device /dev/videoN
cargo run -p omacam-daemon -- ipc snapshot
cargo run -p omacam-output -- --help
cargo run -p omacam-preview -- --probe
./scripts/verify-release.sh
./scripts/start-browser-camera.sh
```

`start-browser-camera.sh` currently expects an existing V4L2 virtual camera
device (default `/dev/video42`) and displays the phone QR code. The browser
requires camera permission and may show a warning for the local self-signed
certificate. Allow the configured port for both TCP and UDP. The PipeWire
backend is selectable in the output worker but the startup script has not yet
been adapted to a device-free PipeWire launch.

`omacam-daemon doctor` is read-only. The user-session `service` owns control/capture state and requires an intentionally selected existing LAN listen address and output device; it does not configure either one. Its D-Bus Start/Stop/Forget/diagnostics intents return operation IDs and expose completion through revisioned snapshots/events. `omacam-output` implements neutral synthetic output plus bounded framed H.264 ingress and never creates or selects a device itself. An optional private `--preview-socket` exposes bounded post-decoder RGBx frames; `omacam-preview` converts that local stream into bounded still frames for the QML panel without another phone stream or H.264 decoder. This path has not yet been physically or live-shell qualified.

The physical pairing and streaming handoff is documented in
[docs/implementation/NEXT-SESSION.md](docs/implementation/NEXT-SESSION.md).
Release qualification still requires the manual and hardware checks in
[docs/implementation/COMPLETION-PLAN.md](docs/implementation/COMPLETION-PLAN.md).

## Safety boundary

OmaCam runtime code must not reconfigure networking, USB functions, ADB, default audio, firewall rules, VPNs, unrelated cameras, or desktop security. Camera capture requires explicit phone consent. See [docs/05-security-and-coexistence.md](docs/05-security-and-coexistence.md).
