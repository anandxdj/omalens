# OmaCam implementation handoff

Updated: 2026-09-08 after implementing the modular authenticated Start/consent, Camera2/MediaCodec, lease, and TLS media vertical slice on top of G4-001. Automated/build/synthetic checks pass; the new path has no physical evidence.

## Product direction

The original product promise is a seamless hybrid phone-camera bridge. Do not treat Android companion installation as universally mandatory. Provider selection should be:

1. use an already available native UVC/Android webcam capability when it satisfies the session;
2. use a zero-install browser/PWA path only after secure-context, local-network permission, signaling, identity, consent, and lifecycle behavior are proven;
3. offer the Android companion as the controlled fallback and advanced-capability provider.

The authoritative PRD, architecture, and roadmap now preserve this exact order. Development ADB installation and terminal commands are test tooling, never the promised customer flow.

## Implemented and verified

- Documentation set under `docs/` reviews the original plan and defines product, architecture, security, state, roadmap, verification, and LLM implementation guidance.
- Rust workspace with `omacam-core`, `omacam-daemon`, and `omacam-output`.
- Independent trust, connection, capture, and output policy; explicit capture consent; generation invalidation; ten-second lease; stale-frame neutralization.
- Read-only desktop readiness diagnostics.
- Synthetic `1280x720@30` YUY2 output proved through a temporary `v4l2loopback` device; the temporary device and capture were cleaned up.
- Android companion debug build and secure QR-pairing vertical slice.
- Pairing invitation: 120-second lifetime, one use, random session/token/challenge, bounded strict JSON, private/link-local endpoint only, desktop certificate binding, no media start.
- Phone: Google code scanner, P-256 identity in Android Keystore, canonical signed transcript, TLS 1.3 exact-certificate pinning, explicit approval.
- Desktop: matching six-digit code, explicit approval, private persistent identity/trust files, status/forget commands, QR removal on completion/expiry/handled cancellation.
- Authenticated restart control uses TLS 1.3 with the saved desktop certificate pin plus a fresh single-use challenge signed by the saved phone identity. Replay, changed key, wrong challenge, expiry, incompatible version, and malformed signature have automated negative coverage.
- `_omacam._tcp.local.` discovery is an untrusted hint tied to the saved certificate fingerprint. The daemon advertises only its explicitly bound address; the Android app starts discovery only after the user taps **Find trusted laptop** and still requires the exact TLS pin.
- The control connection stays open with two-second keepalives and cannot authorize capture. Phone-side Forget clears local trust and sends authenticated revocation; desktop-side `pair forget` is noticed at the next keepalive and tears down the channel.
- Pairing now enforces five failed attempts per source and twenty globally for an invitation.
- `omacam-daemon providers` implements the read-only provider order. On the current host it reports no existing UVC source, browser bootstrap unqualified, and the trusted OnePlus companion ready as fallback.
- G4 desktop ingress foundation: fixed 104-byte framing binds every H.264 access unit to the authenticated phone identity, fresh control connection, media session, and capture generation; it rejects malformed, oversized, stale, replayed, out-of-order, or post-Stop input before decode.
- The preliminary raw `omacam-output --h264-stdin` option was replaced by `--framed-h264-stdin` with explicit non-secret binding identifiers. The constrained-baseline 720p30 OpenH264 path uses a two-buffer leaky decoder queue, fixed YUY2 720p30 normalization, a permanent neutral source, and a 400 ms last-frame repeat limit.
- Development control can explicitly request Start only when paired with an explicit output device. The phone visibly names the laptop and offers Share/Decline; reconnect, discovery, pairing, and the desktop alone still cannot grant capture. The peer/control-connection/media-session/generation binding is completed only after matching phone approval.
- A second TLS 1.3 connection to the same listener repeats exact certificate pinning and fresh saved-phone-key proof, then must match the approved binding before media allocation. Its receive queue is bounded to two records and freshness-biased before feeding G4-001.
- Android has one strict Camera2/MediaCodec path: an advertised 1280×720 plus exact 30 fps tuple, AVC baseline level 3.1 surface encoding, bounded access units, a visible phone Stop, ten-second local monotonic lease, and deterministic owned-resource cleanup. Only `CAMERA` was added; compiled permissions contain no microphone access.
- The daemon is now split into control, capture/media, diagnostics, and remaining CLI/pairing/storage modules; Android control parsing and camera/media ownership are separate from the activity.
- Rust: 51 tests pass (37 core, 9 daemon, 5 output); strict Clippy passes.
- Android: debug assembly succeeds; lint has no issues; APK v2 signature verifies.
- Real pairing succeeded with `OnePlus CPH2661` over its hotspot after an explicit scoped UFW allowance. Desktop trust status confirms the phone identity.

## Important environment evidence

- Host: Omarchy 4.0.2, Quickshell 0.3.1, Linux 7.1.9, GStreamer 1.28.6, PipeWire 1.6.8.
- OnePlus hotspot trial: phone/gateway `10.87.237.176`, laptop `10.87.237.216`, TCP 47123.
- UFW is active and blocked the first inbound connection. OmaCam correctly made no hidden firewall change. A product installer/onboarding design must offer an explicit narrowly scoped rule or a transport architecture that avoids a standing inbound exception, with ownership and removal behavior.
- USB inspection while tethering showed the OnePlus exposing RNDIS and CDC Data only, not UVC. Do not change its USB function automatically or claim UVC/tether coexistence.
- JDK 17 is installed. A temporary Android SDK exists at `/tmp/omacam-android-sdk`; it may disappear after reboot. The built APK is under `android/app/build/outputs/apk/debug/`.
- The current GStreamer installation has `h264parse`, `fdsrc`, `openh264enc`, `openh264dec`, `queue`, `videoconvert`, `videoscale`, `videorate`, `compositor`, `input-selector`, `intervideosink`, `intervideosrc`, `unixfdsink`, and `unixfdsrc`. `avdec_h264` and `fallbackswitch` are missing. The current bounded development topology explicitly selects constrained-baseline OpenH264; do not silently install or substitute another decoder.

## Current limitations

- Pairing, authenticated control, explicit Start/consent, phone capture/encoder code, and authenticated network media receive are implemented, but the control and G4 paths have not passed their required physical OnePlus trials. There is no physical camera/media evidence, preview, durable virtual-camera provisioning, finished QML onboarding, package, or release signing.
- The 400 ms compositor fallback and neutral survival passed only with local synthetic H.264 and `fakesink`. No recognizable-image V4L2 timing, Stop-to-neutral measurement, consumer application, or physical camera result exists for G4.
- The terminal QR and manual UFW command are development workflows, not seamless UX.
- Companion pairing is proven on one device only. The remaining G3 negative/restart/Forget checks are not recorded as passed.
- Current Android scanner depends on Google Play services and is therefore not a universal Android path.
- Public name, application ID, repository ownership, release signing, packaging identifiers, and license remain unresolved.

## Current worktree and validation

The G3/provider and G4 desktop-ingress work is intentionally still uncommitted in the shared worktree. Preserve all existing modifications and untracked implementation records; do not reset or overwrite them. The latest completed checks are:

- `cargo test --workspace`: 51 tests pass (37 core, 9 daemon, 5 output).
- `cargo clippy --workspace --all-targets -- -D warnings`: passes.
- Android `assembleDebug`, `lintDebug`, and `testDebugUnitTest`: succeeds; there are no JVM test sources yet.
- `apksigner verify --verbose`: APK Signature Scheme v2 verifies with one debug signer.
- `git diff --check`: passes.
- `omacam-output --probe`: neutral and H.264 live-input dependencies are ready.
- Compiled APK permission inspection shows CAMERA but no RECORD_AUDIO; APK v2 signature still verifies with one debug signer.
- A fresh constrained-baseline Annex-B synthetic stream decoded and normalized to fixed YUY2 1280×720@30 in `fakesink`. The check fixed an interoperability issue for valid H.264 without SPS frame-rate timing: width/profile remain bounded before decode and `videorate` establishes 30 fps.
- Exact constrained-baseline 1280×720@30 synthetic H.264 negotiated through the decode/normalize/compositor topology into `fakesink`; after media EOF the neutral live branch remained active until the five-second harness timeout. Empty input likewise remained neutral until timeout.
- A real desktop smoke test bound `control serve` to `10.87.237.216:47123`, loaded the saved OnePlus trust identity, and initialized its mDNS advertisement. It was then stopped manually; no firewall/network/USB setting changed.
- The OnePlus was not attached when the latest APK was ready, so no claim is made that the new NSD/restart/revocation path has passed on-device.

## Next recommended work

1. When the OnePlus is available, execute the pending G3 restart/spoof/revocation procedure in `G3-002-authenticated-control.md`, then the G4-002 approve/decline/permission/busy/Start/phone-Stop/desktop-Stop/link-loss/lease-expiry procedure. Do not change firewall, network, USB, tethering, or hotspot state automatically.
2. Feed recognizable phone imagery through an OmaCam-owned V4L2 device and real consumer. Measure every Stop/loss path to neutral, verify the consumer handle persists, and run the required 30-minute trial. Record failures rather than weakening gates.
3. Add Android protocol/lifecycle tests and fault injection for camera/encoder callback and output-shape failures. Keep physical work independent from these automated improvements.
4. Add the shared preview after the existing single decoder, without a second phone stream or a decoder in QML.
5. Continue reducing the remaining daemon entry point into pairing/storage modules as those areas change; preserve the new control/capture/diagnostics boundaries. Then add structured provider/preflight state and native UI. Load the local Omarchy skill before live shell work; no shell restart or config mutation without explicit authorization.

The pairing record is in [G3-001-qr-pairing.md](G3-001-qr-pairing.md). The restart protocol is in [G3-002-authenticated-control.md](G3-002-authenticated-control.md). The bounded ingress is in [G4-001-bounded-media-ingress.md](G4-001-bounded-media-ingress.md), and the new Start/camera/media slice and exact evidence split are in [G4-002-authenticated-start-camera-media.md](G4-002-authenticated-start-camera-media.md).

## Immediate resume command

From a fresh conversation, instruct the agent to read this file plus the PRD, architecture, contracts/state model, security rules, roadmap, verification plan, and every implementation record, then inspect the entire worktree and diff as authoritative. Preserve all modifications and untracked files. Continue the highest-priority coherent unfinished package: run the queued G3/G4 physical trials when the OnePlus is available, otherwise add independent Android protocol/lifecycle/fault tests or the single-decoder shared preview foundation. Preserve explicit consent, binding, Stop-first invalidation, lease, bounds, neutral output, no audio, modular boundaries, and all coexistence prohibitions. Repeat the implementation-record, full verification, accurate implemented/automated/physical/pending checkpoint, and copyable-prompt handoff protocol. Do not mark G3, G4, any roadmap gate, or the MVP complete without required physical and application evidence.
