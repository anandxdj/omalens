# G4-003 — Android protocol and capture-lifecycle hardening

Status: implemented and automated-verified on the JVM/build toolchain; no new physical phone, V4L2, preview, or consumer evidence. G3 and G4 remain open.

Requirements: R05, R06, R09–R11, R13, R15, R20. Gate: partial G4, with additional G1 contract evidence.

## Objective and implemented result

This independent package was selected because the OnePlus was not connected. It adds the first Android JVM sources and makes explicit consent correlation and Android media framing independently testable without relaxing any physical gate.

The phone now owns a synchronized, memory-only `CaptureSessionPolicy` with separate Idle, AwaitingConsent, AwaitingGrant, and Streaming phases. Authenticated control cannot grant capture from Idle or AwaitingConsent. Approval must match the exact pending 128-bit request identifier, a second request cannot replace it, Stop invalidates every phase before cleanup or peer notification, and a fresh activity/process policy begins Idle.

The desktop `capture_granted` response now echoes the exact approved `request_id`. Android strictly requires and decodes that identifier, then matches it against its local AwaitingGrant phase before constructing `CameraStreamer`. This closes a correlation gap where the desktop validated the approval but the phone could only establish that some local request had previously been approved. The existing peer/control-connection/media-session/generation binding remains mandatory and unchanged.

Android media-record serialization moved into a pure `MediaProtocol` boundary used by `CameraStreamer`. Tests compare its H.264 and terminal Stop bytes against the Rust 104-byte big-endian framing contract. Empty and over-1-MiB access units, negative sequence/timestamp values, malformed hex bindings, and zero generations fail before any bytes are written. The encoder still emits no audio and retains its two-sided Stop/lease/resource cleanup behavior.

Android URL-safe Base64 now uses the Java standard-library implementation available below the project's minimum SDK. Encoding remains URL-safe and unpadded; decoding remains strict. This removes an Android framework stub from local protocol tests without adding runtime permissions or network behavior.

## Automated evidence from 2026-09-09

- Android `testDebugUnitTest`: 16 tests pass (8 strict control-message tests, 5 capture-policy tests, 3 media-framing/fault tests); zero failures or errors.
- Control tests cover the supported Start tuple, exact/unknown fields, request-ID length, UTF-8 display-name byte bounds, binding dimensions, positive generation, bounded Stop reasons, oversized messages, and correlated command construction.
- Lifecycle tests cover consent-before-grant, exact request approval, stale grant rejection, second-request exclusion, duplicate-grant rejection, decline, Stop while awaiting a grant, late post-Stop grant rejection, and restart-to-Idle behavior.
- Media tests verify all header offsets, identifiers, generation, sequence, timestamp, flags, payload length, terminal Stop shape, and pre-write rejection for invalid encoder/binding shapes.
- Rust `cargo test --workspace`: 52 tests pass (37 core, 10 daemon, 5 output), including a new desktop serialization check for exact `capture_granted.request_id` echo.
- Strict workspace Clippy, Rust formatting, Android `assembleDebug`, `lintDebug`, and `git diff --check` pass.
- APK Signature Scheme v2 verifies with one debug signer. Compiled permissions contain `CAMERA` but no microphone permission.
- `omacam-output --probe` reports neutral and H.264 dependencies ready.
- A fresh constrained-baseline 1280×720@30 synthetic stream passed the bounded OpenH264 decode/normalize/compositor topology into `fakesink`; after source EOF the neutral branch remained active until the five-second harness timeout. An empty-input neutral run remained active until its three-second timeout.

## Physical and application status

Not tested in this package. The phone was explicitly reported unavailable over USB, so no ADB, camera, restart, spoofing, identity-change, Forget, consent, permission, busy-camera, Stop, link-loss, or lease trial was attempted. No network, firewall, route, DNS, VPN, USB function, tethering, hotspot, shared video device, desktop shell, or unrelated process was changed.

There is still no physical proof that Camera2 opens on the OnePlus, MediaCodec interoperates with the desktop, recognizable imagery reaches an OmaCam-owned V4L2 device, neutral replacement occurs within 500 ms, a consumer handle persists, the phone releases by ten seconds, a real application receives frames, or the required sustained run succeeds.

## Pending work

1. Run the G3-002 restart/spoof/identity-change/Forget trials when the OnePlus is reachable, then all G4-002 physical consent/permission/busy/Stop/link-loss/lease trials.
2. Feed recognizable phone imagery through an OmaCam-owned V4L2 device and real consumer. Measure every Stop/loss path to neutral, consumer-handle persistence, and the 30-minute G4 trial.
3. Add fault-injected camera/encoder ownership tests at an instrumented or hardware layer; JVM policy/framing tests cannot establish Camera2 or MediaCodec behavior.
4. Add the single-decoder shared preview foundation without a second stream or QML decoder.
5. Implement persistent, owned virtual-camera provisioning and finish the native service/UI/packaging paths.

G3, G4, G0–G7, and the MVP are not complete.
