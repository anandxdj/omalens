# G4-004 — Shared-preview and deterministic-cleanup foundations

Status: producer, isolated consumer, and QML preview wiring are implemented and automated/synthetic verified without a phone; no live-shell, physical camera, V4L2 consumer, or application evidence. G4 remains open.

Requirements: R05, R09–R11, R13, R20. Gate: partial G4, with foundations for G5 and G6.

## Objective and implemented result

The phone was unavailable, so this package implements safe no-phone follow-ups identified by the prior handoff: a single-decoder shared-preview boundary, an isolated bounded preview consumer with panel wiring, and fault-isolated Android resource cleanup.

`omacam-output` now accepts an optional terminal `--preview-socket` argument. After the existing H.264 decoder, neutral/live compositor, and stale-frame policy, one `tee` feeds independently leaky output and preview queues. V4L2 remains fixed YUY2 1280×720 at 30 fps. Preview is bounded to one RGBx 640×360 frame at 15 fps and is exported by GStreamer's local Unix-FD shared-memory transport. The sink does not wait for a client, so closing or slowing a preview cannot block the consumer output or control path. There is still exactly one phone stream and one H.264 decoder.

The preview endpoint writes no frame file and is opt-in. It must be an absolute, nonexistent path below an existing non-symlink directory with no group/other permissions; the output worker refuses to replace any existing path. The development control command accepts it only together with explicit `--request-start` and `--output-device`.

The new `omacam-preview` process waits for the exact private runtime socket, consumes only post-decoder RGBx frames, keeps a one-frame leaky queue, reduces delivery to 10 fps, JPEG-encodes outside the shell, rejects JPEGs over 256 KiB, and emits one base64 frame per line. Its fixed GStreamer child receives a parent-death signal and controls no device or network input. The QML panel starts this helper only while open, rejects empty or oversized lines, retains only the newest frame when its `Image` is not loading, and clears/kills preview on close. QML decodes bounded JPEG stills, not H.264 or network video. Actual loading in the long-lived Omarchy shell remains untested and is not claimed.

Android camera/codec cleanup now snapshots and clears owned references before release, then runs every cleanup step in order even if an earlier platform call throws. Session stop/abort/close, camera close, encoder stop/release, surface release, TLS socket close, and handler-thread shutdown/join are isolated from one another. This preserves repeated-cleanup safety and prevents one faulty callback from skipping later resources. Pure JVM fault tests cover ordered continuation and empty cleanup; Camera2/MediaCodec callbacks themselves still require instrumented or physical evidence.

## Verification performed on 2026-09-09

- `cargo test --workspace`: 62 tests pass (37 core, 13 daemon, 8 output, 4 preview).
- `cargo clippy --workspace --all-targets -- -D warnings`: passes.
- `cargo fmt --check` and `git diff --check`: pass.
- `omacam-output --probe`: neutral output, H.264 live input, and shared-preview dependencies are ready.
- `omacam-preview --probe`: the isolated preview-consumer dependencies are ready.
- Rust tests prove that preview CLI scope is explicit, existing paths cannot be replaced, and the generated graph contains exactly one `openh264dec`, one shared `tee`, a one-buffer leaky preview queue, fixed preview caps, and a non-waiting Unix-FD sink.
- Preview tests cover JPEG framing/noise, truncated and over-256-KiB rejection, independent base64 lines, the one-frame leaky consumer graph, and the absence of any H.264 decoder.
- A real local GStreamer Unix-FD producer/consumer trial passed 30 RGBx 640×360 preview frames.
- A constrained-baseline 1280×720 H.264 synthetic source passed the exact single-decoder/compositor/dual-queue topology; a Unix-FD client consumed 30 preview frames while the independent output branch remained active in `fakesink`.
- The built `omacam-preview` helper waited for a later-created private socket and delivered three bounded base64 JPEG frames end to end. Its downstream-close path exited cleanly.
- `qmllint -I /usr/share/omarchy/shell omarchy-plugin/BarWidget.qml omarchy-plugin/Panel.qml`: passes with the preview wiring.
- Android `testDebugUnitTest`: 18 tests pass (the prior 16 plus 2 cleanup-fault tests). Debug assembly completes. Lint completes with one dependency-version warning for the existing JVM-only `org.json` test fixture and no errors.
- The debug APK verifies with APK Signature Scheme v2 and one debug signer. Compiled permissions contain `CAMERA` but no `RECORD_AUDIO`.

## Security, privacy, and coexistence review

No phone, camera, microphone, live V4L2 device, network route, firewall, DNS, VPN, USB function, tethering, hotspot, shared video device, desktop configuration, shell process, or unrelated process was changed. Synthetic socket tests used private temporary directories and were removed. No media was retained.

The Omarchy skill and plugin guide were read before shell-facing investigation. Packaged shell sources were read only. No live shell/plugin installation, configuration edit, or restart was performed.

## Physical, application, and pending status

Physical: not tested. Application: not tested. The OnePlus, a real OmaCam-owned V4L2 device, actual Quickshell preview, and consumer apps were unavailable or intentionally not touched.

Next work is to load the preview path in an isolated/live Omarchy shell test when explicitly authorized, verify buffer/process cleanup and shell stability, and replace development CLI orchestration with bounded service IPC. In parallel-safe Android work, add callback-level startup/cancellation fault injection around Camera2/MediaCodec acquisition. The queued G3/G4 phone trials and all V4L2/application timing and sustained tests remain mandatory when hardware is available.

G0–G7, G3, G4, the MVP, and the full product are not complete.
