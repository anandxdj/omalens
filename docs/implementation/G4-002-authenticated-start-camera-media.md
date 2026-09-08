# G4-002 — Authenticated Start, camera, and media vertical slice

Status: implementation and automated verification complete; no physical phone, V4L2 consumer, preview, or application evidence exists for this package. G4 remains open.

Requirements: R05, R06, R08–R11, R13, R15, R20. Gate: partial G4 only.

## Objective and implemented result

This package connects the G3 authenticated control channel to the bounded G4-001 ingress without allowing trust, discovery, reconnect, or the desktop alone to grant capture.

The development desktop command now accepts `--request-start --output-device /dev/videoN` only as a pair. After fresh TLS 1.3 phone-key authentication, it sends one exact 1280×720@30 constrained-baseline Start request naming the laptop. The phone presents a visible **Share camera** / **Decline** decision. Camera permission is requested only after Share is tapped. A declined or denied request mints no capture binding and opens no camera.

Only a matching approval for the fresh 128-bit request identifier advances the independent session policy. The desktop then creates the unpredictable media-session identifier and capture generation and combines them with the authenticated phone-key digest and fresh control-connection identifier. A second TLS 1.3 connection to the same explicitly bound listener repeats certificate pinning and fresh P-256 phone-key proof, then must present that exact binding before the server allocates or accepts a media payload. This uses the maintained rustls/Android TLS providers; there is no plaintext fallback, extra discovery authority, public relay, STUN/TURN, or reusable media credential.

The daemon validates the 104-byte G4-001 header before allocating its at-most-1-MiB payload. A two-record Tokio channel drops obsolete video under pressure and preserves freshness; the daemon renumbers forwarded records contiguously for the isolated `omacam-output` process. Malformed, wrong-peer, wrong-connection, wrong-session, stale-generation, replayed, out-of-order, oversized, or timed-out input terminates the capture. The output worker remains alive on its permanent neutral branch after terminal Stop until the authenticated control session ends.

The Android pipeline is split into `ControlProtocol.kt` and `CameraStreamer.kt`, separate from activity/UI orchestration. It selects only a camera advertising 1280×720 and an exact 30–30 fps range, configures `MediaCodec` AVC baseline level 3.1 at 4 Mbit/s with a surface input, prefixes codec configuration to keyframes, and sends bounded Annex-B access units. Unsupported capability, permission denial, busy/error callbacks, setup timeout, transport failure, and lifecycle loss release partial resources visibly. Camera, capture session, encoder, surface, socket, and handler thread are owned by one object and released deterministically.

## Stop, lease, and privacy behavior

- Phone Stop and activity pause invalidate the local active flag first, attempt a terminal framed Stop, close transport, and release all owned camera/encoder resources.
- Desktop Stop paths invalidate `SessionPolicy` first, write terminal Stop into G4-001, and leave the writer neutral rather than freezing the last frame.
- In the development CLI, Ctrl-C during active capture is desktop Stop: it invalidates locally, sends authenticated `stop_capture`, writes terminal ingress Stop, and keeps the neutral writer/control service alive. A later Ctrl-C while idle shuts down the service.
- Authenticated phone pings arrive every two seconds. Active pongs refresh both sides' local monotonic authorization deadlines; ten seconds without valid control disarms capture.
- The G4-001 compositor still limits captured-frame repetition to 400 ms, leaving scheduling margin under the 500 ms neutral-output requirement.
- The manifest adds only `CAMERA` for capture. Compiled permissions contain no `RECORD_AUDIO`; there is no audio API, track, source, or microphone capture.

## Modularity work

The daemon's previous monolithic entry point was reduced and split into `control_server.rs`, `capture.rs`, and `diagnostics.rs`; `main.rs` retains CLI/pairing/storage orchestration. Android control parsing and camera/media ownership are separate source files. No existing change or untracked record was discarded or duplicated.

## Automated and synthetic evidence from 2026-09-08

- `cargo test --workspace`: 51 tests pass (37 core, 9 daemon, 5 output).
- Strict workspace Clippy and `cargo fmt --check` pass.
- Tests cover exact Start correlation, the paired Start/output CLI requirement, unknown control fields, malformed media-open bindings, pre-allocation header validation, 1 MiB bounds, peer/connection/session/generation mismatch, sequence/replay, terminal Stop, post-Stop rejection, stale frames, ten-second lease expiry, and second-session exclusion.
- Android `assembleDebug`, `lintDebug`, and `testDebugUnitTest` succeed. The JVM test task still has no sources; Kotlin behavior here is build/lint-verified, not unit-tested.
- Compiled APK permission inspection shows `INTERNET`, multicast discovery, `CAMERA`, AndroidX network state, and generated receiver protection only; no microphone permission. APK Signature Scheme v2 verifies with one debug signer.
- `omacam-output --probe` reports neutral and H.264 dependencies ready.
- A locally generated 1280×720 constrained-baseline Annex-B stream decoded through OpenH264, the two-buffer leaky queue, conversion, and fixed YUY2 720p30 normalization into `fakesink`. This check identified and fixed rejection of valid streams without SPS frame-rate VUI: input dimensions/profile remain constrained before decode, and `videorate` owns the output rate.
- `git diff --check` passes.

## Physical and application status

Not tested. The OnePlus was unavailable for this package. No claim is made that Camera2 opens, the hardware encoder honors baseline, TLS media interoperates on-device, a recognizable image reaches V4L2, Stop reaches neutral within 500 ms, the phone lease releases within ten seconds, a meeting app consumes frames, or a 30-minute run succeeds.

## Pending work

1. Run the G3 restart/spoof/revocation trial and this G4 Start/approve/decline/permission/busy/Stop/lease procedure on the paired OnePlus without changing network, firewall, USB, tethering, or hotspot state automatically.
2. Measure recognizable-image Stop and loss timing through an OmaCam-owned V4L2 device and a real consumer; verify the writer/consumer handle remains stable.
3. Add Android protocol/lifecycle unit or instrumented tests where they provide evidence beyond compilation; add fault tests for camera callbacks and encoder output shapes.
4. Add one shared desktop preview sourced after the single decoder. Do not add another phone stream or shell decoder.
5. Replace the development CLI/process wiring with bounded owned service IPC while preserving this consent and binding protocol.

G3, G4, G0–G7, and the MVP are not complete.
