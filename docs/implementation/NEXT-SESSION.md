# OmaCam implementation handoff

## 2026-09-09 coordinated-session stop handoff

The user stopped the multi-agent run to conserve tokens. All agents were
interrupted or already complete; preserve the dirty tree exactly as it now
stands. This subsection supersedes older test counts below where they differ.

### Work completed in this session

- Android added an `AndroidJUnitRunner` configuration and
  `CaptureLifecycleInstrumentedTest.kt` with four device-runnable tests for
  invalidation-before-cleanup, stale Camera2/session callbacks across a fresh
  generation, continuation through ten throwing cleanup steps, and duplicate
  terminal camera callbacks. The detailed record is G4-005.
- Desktop service changes synchronously invalidate Stop/Forget authorization,
  cancel pending Start operations, turn a closed intent channel into a typed
  `service_unavailable` failure, clear pending phone consent on Stop, and drain
  bounded media backlog toward fresher access units while retaining terminal
  priority. Three daemon regression tests were added.
- The Omarchy repository plugin was expanded with authoritative schema-2
  snapshot rendering, revision-event refresh, periodic recovery, bounded
  post-decoder preview, provider/onboarding guidance, diagnostics, Start/Stop,
  two-step Forget, loading/disabled/error states, and keyboard/accessibility
  affordances. The Omarchy skill was read first. No live shell/config was
  changed or restarted. This agent was interrupted before sending a final
  report, so re-run QML lint and inspect the full QML diff next session.
- Packaging gained hardened user-unit state ownership, explicit packaging and
  uninstall documentation, and a temporary-root install/upgrade/rollback/
  uninstall lifecycle test. The isolated lifecycle script passed against the
  release binaries and preserved simulated user data and unrelated files.

### Evidence actually executed in this session

- Android agent: `testDebugUnitTest --rerun-tasks` passed 29 JVM tests;
  `assembleDebug`, `assembleDebugAndroidTest`, `assembleRelease`, `lintDebug`,
  and `lintRelease` passed under JDK 17 and `/tmp/omacam-android-sdk`.
  Debug APK v2 verification passed with one debug signer. Permission inspection
  found CAMERA/INTERNET/multicast/network-state and no RECORD_AUDIO.
- Desktop agent: `cargo test --workspace --all-targets` passed 66 tests
  (37 core, 17 daemon, 8 output, 4 preview); strict all-target Clippy, Rust
  formatting, and `git diff --check` passed.
- Coordinator: `cargo build --release --locked --workspace` passed;
  `packaging/tests/install-lifecycle.sh` passed; `makepkg --printsrcinfo`
  passed; `qmllint omarchy-plugin/*.qml` passed before the final interrupted
  QML edits; `adb devices -l` returned no devices. The two host video nodes are
  the integrated webcam and were intentionally not opened or modified.
- A real `makepkg --force --noconfirm --cleanbuild` was attempted and stopped at
  dependency resolution: `gst-plugin-openh264` does not exist as an installed
  package on this Arch host. `/usr/lib/gstreamer-1.0/libgstopenh264.so` is owned
  by `gst-plugins-bad 1.28.6-2`, which is already declared. Correct the package
  dependency, then rerun the package build and lifecycle checks next session.

### Critical integration finding

The output worker is not genuinely service-scoped. Current work keeps it
neutral only until the authenticated control connection ends, and the attempted
post-Stop loop can stop consuming D-Bus intents. A correct persistent consumer
handle across Stop/reconnect requires moving output ownership above the control
connection and defining a bounded reset/rebind protocol without allowing an old
session/generation into the decoder. Do not claim persistent V4L2 lifecycle or
G4/G5 completion until this is implemented and tested.

### Exact next-session start

1. Re-read this section, inspect the complete diff/untracked files, and run
   `git diff --check` plus `qmllint omarchy-plugin/*.qml`.
2. Fix the Arch dependency metadata (`openh264` is supplied by
   `gst-plugins-bad` on the recorded host), build the package, inspect its file
   list, and rerun the isolated lifecycle test.
3. Resolve the service-scoped output ownership/reset protocol and add tests for
   Stop/reconnect, stale generations, D-Bus intents after Stop, worker crash,
   and cleanup continuation. Then run the complete Rust suite yourself.
4. With an Android device/emulator, run:
   `JAVA_HOME=/usr/lib/jvm/java-17-openjdk ANDROID_HOME=/tmp/omacam-android-sdk
   ./android/gradlew -p android connectedDebugAndroidTest`. Physical Camera2,
   permission, busy-camera, media, V4L2, sustained, and live-shell evidence
   remains open.

No gate, MVP, packaging release, or full product was closed in this stopped
session.

Updated: 2026-09-09 after adding generation-safe Android Camera2 callbacks and guarded MediaCodec/resource acquisition with deterministic startup/cancellation fault injection. Automated/static/synthetic checks pass; the camera path and UI still have no new instrumented, physical, live-shell, or application evidence.

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
- Android now has a synchronized memory-only capture policy. A grant is rejected unless the exact locally offered 128-bit Start request was explicitly approved; Stop invalidates pending approval or streaming state before cleanup, and restart begins Idle.
- `capture_granted` now echoes the exact approved request identifier in addition to the existing peer/control-connection/media-session/generation binding. Android requires both correlations before camera startup.
- Android media framing is isolated and byte-tested against the Rust 104-byte contract. Invalid encoder shapes and malformed bindings fail before output.
- The existing decoded/neutral compositor can now tee into an optional local Unix-FD shared-preview boundary without another phone stream or H.264 decoder. Preview is fixed RGBx 640×360@15, bounded to one leaky frame, does not wait for a client, writes no frame file, and refuses unsafe or existing socket paths.
- A separate `omacam-preview` helper consumes only that decoded stream, keeps a one-frame leaky queue, delivers at most 10 fps, bounds JPEGs to 256 KiB, and sends base64 still frames to QML. The panel keeps only the latest non-loading image and starts/stops the helper with panel visibility. H.264 and network media remain outside QML. Live-shell behavior is not tested.
- Android resource cleanup snapshots and clears ownership before running every release step independently, so a failing Camera2/MediaCodec cleanup call cannot skip later resources.
- Android Camera2 callbacks and MediaCodec/resource acquisition now share a monotonic generation gate. Stop/cancellation invalidates before cleanup; late camera/session or acquisition completion closes the resource instead of publishing it. Deterministic JVM fault injection covers busy/error, disconnect, session failure, partial acquisition, late callbacks, lease-style cancellation, and single-attempt acquisition failure.
- A versioned `dev.omacam.Session1` session-bus service exposes bounded JSON snapshots/diagnostics plus service-owned lifecycle intents. No preview frames or secrets cross D-Bus.
- The session service now owns the authenticated control/capture runtime instead of synthesizing snapshots. Schema-2 snapshots include monotonic revision, 32 bounded operation results, and typed last-error state. Bounded `RequestStart`, `Stop`, `ForgetPeer`, and diagnostics intents are idempotent by operation ID; Start is only accepted for trusted/current connections and is not successful until exact phone consent/binding plus first authenticated media establish streaming.
- Revisioned `StateChanged` events feed an `ipc events` watcher. The panel refreshes full snapshots after each event and uses periodic full refresh if events are missed/interrupted. Start/Stop controls and errors reflect service truth; QML still receives only bounded post-decoder preview stills.
- Stop and Forget publish invalidated authorization before cleanup. The operation/intent queues remain bounded, and repeated Stop is safe.
- A prototype systemd user unit and Arch `PKGBUILD` install fixed-purpose artifacts without enabling/starting the service, running arbitrary scripts, or changing network/device configuration. Installation, V4L2 provisioning, package lifecycle, and release identity remain unproven.
- Rust: 63 tests pass (37 core, 14 daemon, 8 output, 4 preview); strict Clippy passes.
- Android: 29 JVM tests pass; debug assembly succeeds; lint has no errors and one existing test-dependency version warning; APK verification is recorded below.
- Real pairing succeeded with `OnePlus CPH2661` over its hotspot after an explicit scoped UFW allowance. Desktop trust status confirms the phone identity.

## Important environment evidence

- Host: Omarchy 4.0.2, Quickshell 0.3.1, Linux 7.1.9, GStreamer 1.28.6, PipeWire 1.6.8.
- OnePlus hotspot trial: phone/gateway `10.87.237.176`, laptop `10.87.237.216`, TCP 47123.
- UFW is active and blocked the first inbound connection. OmaCam correctly made no hidden firewall change. A product installer/onboarding design must offer an explicit narrowly scoped rule or a transport architecture that avoids a standing inbound exception, with ownership and removal behavior.
- USB inspection while tethering showed the OnePlus exposing RNDIS and CDC Data only, not UVC. Do not change its USB function automatically or claim UVC/tether coexistence.
- JDK 17 is installed. A temporary Android SDK exists at `/tmp/omacam-android-sdk`; it may disappear after reboot. The built APK is under `android/app/build/outputs/apk/debug/`.
- The current GStreamer installation has `h264parse`, `fdsrc`, `openh264enc`, `openh264dec`, `queue`, `videoconvert`, `videoscale`, `videorate`, `compositor`, `input-selector`, `intervideosink`, `intervideosrc`, `unixfdsink`, and `unixfdsrc`. `avdec_h264` and `fallbackswitch` are missing. The current bounded development topology explicitly selects constrained-baseline OpenH264; do not silently install or substitute another decoder.

## Current limitations

- Pairing, authenticated control, explicit Start/consent, phone capture/encoder code, authenticated network media receive, and the shared-preview producer/consumer/panel wiring are implemented, but the control and G4 paths have not passed their required physical OnePlus or live-shell trials. There is no physical camera/media evidence, rendered live preview, durable virtual-camera provisioning, finished onboarding, package, or release signing.
- The 400 ms compositor fallback and neutral survival passed only with local synthetic H.264 and `fakesink`. No recognizable-image V4L2 timing, Stop-to-neutral measurement, consumer application, or physical camera result exists for G4.
- The terminal QR and manual UFW command are development workflows, not seamless UX.
- Companion pairing is proven on one device only. The remaining G3 negative/restart/Forget checks are not recorded as passed.
- Current Android scanner depends on Google Play services and is therefore not a universal Android path.
- Public name, application ID, repository ownership, release signing, packaging identifiers, and license remain unresolved.

## Current worktree and validation

At the start of the 2026-09-09 session, `main` was clean at `33f4386`; the older statement that G3/G4 were uncommitted was stale. The current shared worktree now intentionally contains the G4-003 Android policy/protocol changes, tests, and implementation record. Preserve all modifications and untracked files; do not reset or overwrite them. The latest completed checks are:

- `cargo test --workspace`: 63 tests pass (37 core, 14 daemon, 8 output, 4 preview).
- `cargo clippy --workspace --all-targets -- -D warnings`: passes.
- `cargo fmt --check` and `git diff --check`: pass.
- Android `assembleDebug`, `lintDebug`, and `testDebugUnitTest`: succeed; 29 JVM tests pass (8 control protocol, 5 capture policy, 3 media framing, 2 resource-cleanup fault, 6 generation/acquisition gate, and 5 callback fault tests). Lint has no errors and reports one newer-version warning for the existing JVM-only `org.json` fixture.
- `apksigner verify --verbose`: APK Signature Scheme v2 verifies with one debug signer.
- `git diff --check`: passes.
- `omacam-output --probe`: neutral, H.264 live-input, and shared-preview dependencies are ready.
- `omacam-preview --probe`: the isolated preview-consumer dependencies are ready.
- `dbus-run-session` smoke-tested `omacam-daemon service` plus both IPC client methods; replies remained bounded and JSON-structured.
- A fresh schema-2 IPC smoke verified bounded snapshots, diagnostics intent completion, repeated Stop idempotency, and typed `peer_not_trusted` Start rejection. The service emitted revisioned state signals; the panel consumes them as full-snapshot refresh hints.
- `qmllint`, systemd unit parsing, and Arch `.SRCINFO` generation pass. The unit verifier reports only the expected warning that the prototype `/usr/bin/omacam-daemon` is not installed on this worktree host.
- Compiled APK permission inspection shows CAMERA but no RECORD_AUDIO; APK v2 signature still verifies with one debug signer.
- A fresh constrained-baseline Annex-B synthetic stream decoded and normalized to fixed YUY2 1280×720@30 in `fakesink`. The check fixed an interoperability issue for valid H.264 without SPS frame-rate timing: width/profile remain bounded before decode and `videorate` establishes 30 fps.
- Exact constrained-baseline 1280×720@30 synthetic H.264 negotiated through the decode/normalize/compositor topology into `fakesink`; after media EOF the neutral live branch remained active until the five-second harness timeout. Empty input likewise remained neutral until timeout.
- A private local Unix-FD synthetic trial delivered 30 fixed RGBx 640×360@15 preview frames. A second trial used the exact constrained-baseline single-decoder/compositor topology and delivered 30 preview frames while the separately queued output branch stayed active in `fakesink`. Temporary sockets and media were removed.
- The built preview helper waited for a private socket, then delivered three bounded base64 JPEG frames end to end and exited cleanly when its consumer closed. QML lint passes for panel lifecycle and latest-frame wiring; the long-lived shell was not started or restarted.
- A real desktop smoke test bound `control serve` to `10.87.237.216:47123`, loaded the saved OnePlus trust identity, and initialized its mDNS advertisement. It was then stopped manually; no firewall/network/USB setting changed.
- The OnePlus was not attached when the latest APK was ready, so no claim is made that the new NSD/restart/revocation path has passed on-device.

## Next recommended work

1. When the OnePlus is available, execute the pending G3 restart/spoof/revocation procedure in `G3-002-authenticated-control.md`, then the G4-002 approve/decline/permission/busy/Start/phone-Stop/desktop-Stop/link-loss/lease-expiry procedure. Do not change firewall, network, USB, tethering, or hotspot state automatically.
2. Feed recognizable phone imagery through an OmaCam-owned V4L2 device and real consumer. Measure every Stop/loss path to neutral, verify the consumer handle persists, and run the required 30-minute trial. Record failures rather than weakening gates.
3. Run instrumented callback timing and permission/busy tests on Android hardware when available. JVM fault injection now covers OmaCam's generation and ownership decisions, but it does not replace framework/driver evidence.
4. Extend the owned service/output lifecycle only with real evidence: persistent neutral writer supervision and intentional onboarding/provisioning remain open. Do not turn accepted intents into premature success or add automatic network/device repair.
5. Verify the existing isolated preview consumer and QML wiring in a controlled shell test when explicitly authorized. Measure update/buffer behavior, close/reopen cleanup, and shell stability without adding another phone stream or H.264 decoder or persisting frames. Continue native onboarding/UI and provider/preflight detail.
6. Continue reducing the remaining daemon entry point into pairing/storage modules as those areas change. Load the local Omarchy skill before live shell work; no shell restart or config mutation without explicit authorization.

The pairing record is in [G3-001-qr-pairing.md](G3-001-qr-pairing.md). The restart protocol is in [G3-002-authenticated-control.md](G3-002-authenticated-control.md). The bounded ingress is in [G4-001-bounded-media-ingress.md](G4-001-bounded-media-ingress.md), the Start/camera/media slice is in [G4-002-authenticated-start-camera-media.md](G4-002-authenticated-start-camera-media.md), Android protocol/lifecycle hardening is in [G4-003-android-protocol-lifecycle-tests.md](G4-003-android-protocol-lifecycle-tests.md), preview/cleanup foundations are in [G4-004-shared-preview-and-cleanup-foundations.md](G4-004-shared-preview-and-cleanup-foundations.md), callback fault injection is in [G4-005-android-callback-fault-injection.md](G4-005-android-callback-fault-injection.md), and service ownership/intents are in [G5-002-owned-session-service-intents.md](G5-002-owned-session-service-intents.md).

## Immediate resume command

Continue OmaCam from the existing dirty worktree after reading NEXT-SESSION.md, all baseline docs, all implementation records, the complete diff, and all untracked files.

Run Android instrumented and physical callback tests for permission denial, camera busy, Stop, disconnect, lease expiry, partial acquisition, and late Camera2 callbacks. Exercise recognizable imagery through the existing one-stream/one-decoder path.

Prove:

- real framework callbacks obey the JVM-proven generation rules;
- every Stop/loss path reaches neutral within the documented bound;
- camera resources release within ten seconds;
- the consumer handle persists and preview close/reopen is stable;
- no coexistence setting changes and no extra stream, decoder, or audio permission appears.

Run all automated/static/synthetic verification, update implementation records and NEXT-SESSION.md, preserve all dirty work, and keep physical/application/release gates open until actually tested.

## Physical-phone continuation prompt

Continue OmaCam from the existing dirty worktree. Read NEXT-SESSION.md, every baseline document, every implementation record, the full diff, and every untracked file first. Preserve everything.

Run and record all remaining G3 and G4 physical tests: restart, spoofed discovery/certificate, identity changes, phone Forget, desktop Forget, active-channel revocation, Start approval/decline, permission denial, camera busy, phone Stop, desktop Stop, link loss, lease expiry, recognizable imagery through the OmaCam-owned V4L2 device, neutral fallback within 500 ms, consumer-handle persistence, camera release within ten seconds, preview close/reopen, and sustained 30/60-minute qualification.

Do not automatically change firewall, routes, DNS, VPN, USB functions, tethering, hotspot, shared video devices, or unrelated processes. Record failures honestly. Run the complete verification suite afterward, update all implementation records and NEXT-SESSION.md, and close gates only when their documented evidence requirements genuinely pass.
