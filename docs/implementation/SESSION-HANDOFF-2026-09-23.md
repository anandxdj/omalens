# OmaCam session handoff — 2026-09-23

The shared worktree is dirty. Preserve all pre-existing and newly created
changes; do not reset, clean, or commit them without reviewing ownership. A
recoverable pre-work baseline is at
`/data/user-data/Projects/omacam-baseline-QRjmB1/` (`tracked.patch` and
`untracked.tar.gz`). The intended architecture and qualification gates are in
`2026-09-23-browser-control-and-modularity.md`.

## What changed

- `omacam-output` was divided into focused CLI/configuration/pipeline/decoder/
  service/preview/mux/test modules. It now has selectable V4L2 and experimental
  PipeWire Video/Source service outputs, with one H.264 decoder and a bounded
  post-decode preview branch.
- Browser signaling was divided into `browser/{options,session,http,rtc,preview}`
  behind a thin `browser_server.rs`. It has phone and desktop bearer roles,
  capability/applied-state and command acknowledgements, Stop, heartbeat expiry,
  and a decoded desktop preview endpoint.
- `webapp/` now contains an Omarchy-inspired phone page and desktop control room,
  with shared API/control helpers and role-specific scripts. The phone has
  browser camera constraints, a black dim mode, wake lock, and Stop lifecycle.
- The Android companion and native service now have Camera2 capability/control
  reporting and acknowledged zoom/exposure/torch/camera/dim/Stop messages over
  the existing authenticated connection on port 47123. Native remains 720p30.
- The shared contract is in `crates/omacam-core/src/camera.rs`.

## Verified here

- `cargo test --workspace --all-targets --locked`: passed after worker stop
  (52 core, 44 daemon, 20 output, 4 preview tests = 120 total).
- `node --test webapp/tests/*.test.js`: 10 tests passed after worker stop.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo fmt --all -- --check`, and `git diff --check`: passed after a small
  test-import lint fix.
- No connected Android device (`adb devices` empty); no Android SDK installed.
  Android build, physical control behavior, PipeWire consumer compatibility,
  and 100-transition optical latency qualification remain unverified.

## Review findings to resolve next

1. Re-run Rust and Node tests, format, strict Clippy, and `git diff --check`
   after any further edits. The recorded checks above passed on the stopped
   worker snapshot.
2. Browser `CameraSession::accept_heartbeat` was changed to clear a pending
   command and report an adjusted/rejected outcome when measured applied
   settings differ. The 1080p-requested/720p-applied regression test passes;
   verify the desktop UI renders that outcome clearly in a real session.
3. The output format is fixed at session start and the server now rejects live
   format changes. Both UIs still expose resolution/FPS controls in a confusing
   state; implement a coherent pre-start selection or explicit Stop/restart
   flow. Do not promise seamless renegotiation.
4. A static visual check found both `<main>` roles initially hidden. A
   `revealRole` call was added to both entry points and a unit test passes; do
   a final real-browser visual check with missing/valid bearer routes.
5. Native Camera2 FPS range detection was corrected to accept any range
   containing 30, including 15–30; the native worker added tests, but Android
   build/JVM execution still requires an SDK. Recheck on hardware.
6. The native output preview socket was changed to refuse any occupied path,
   with a regression test. Verify the browser runtime directory is private
   (0700), and inspect cleanup for ownership-safe deletion.
7. The new desktop browser UI is for the browser provider only. Native control
   is exposed through D-Bus but has no browser UI bridge. Decide and implement
   the unified control-room integration before claiming provider parity.
8. `scripts/start-browser-camera.sh` still requires `/dev/video42`, even if the
   experimental PipeWire backend is selected. Give PipeWire a device-free
   launcher path only after validating the daemon/output option boundary.
9. Complete visual, handset, media, failure, thermal, and optical qualification.
   Do not advertise “Fastest wireless” or a production-ready PipeWire camera
   until measured and consumer-tested.

The current Orca orchestration run was `run_5fb63ecb30b4`. Its Luna xhigh
workers owned separate browser Rust, browser UI, native, and output scopes.
The output and native workers completed. Browser Rust and browser UI follow-up
workers were stopped for this user-requested session handoff after their
filesystem changes had been saved; no agent should keep editing in the
background. Review the stopped dispatches (`ctx_c824ea1e553b` and
`ctx_c7abd0df7a01`) and current files before any new delegation.
