# G4-005 — Android callback and acquisition fault injection

## 2026-09-09 resumed verification

The resumed integration added a bounded `MediaCodecOutput` copy boundary and
closed a Start/Stop race by publishing and starting the streamer under the
capture lifecycle lock while generation-gating terminal callbacks. The
coordinator reran the full JDK-17 matrix: 32 JVM tests passed with zero
failures/errors; debug, instrumentation-test APK, unsigned release APK,
`lintDebug`, and `lintRelease` all built in 126 executed Gradle actions. Debug
APK v2 signing verified with one debug signer, and compiled permissions contain
CAMERA, INTERNET, multicast/network state, and no RECORD_AUDIO. `adb devices
-l` returned no devices, so connected instrumentation and physical framework,
permission, camera-busy, codec-failure, Stop/restart, and revocation evidence
remain open.

Status: implemented and JVM/build/static verified on 2026-09-09 without a phone. A four-test Android instrumentation suite is compiled and ready to run, but `adb devices -l` reported no device/emulator, so no instrumented-device or physical-camera test was executed. Camera2 and MediaCodec behavior is fault-injected at the application boundary. This does not complete G3, G4, the MVP, or the product.

Requirements: R05, R09–R11, R13, R20. Gate: partial G4.

## Implemented

- `CaptureCallbackGate` issues monotonic in-process capture generations. Stop/cancellation invalidates a generation under the same lock used to publish callback-owned resources.
- Camera2 `onOpened`, `onDisconnected`, `onError`, `onConfigured`, and `onConfigureFailed` now delegate through a testable callback boundary. A callback from a stopped generation cannot republish a camera or session and closes its late resource immediately.
- MediaCodec encoder, input-surface, and camera-handler-thread acquisition use the same guarded acquisition boundary. Cancellation is checked between configure, surface creation, codec start, camera open, session creation, and repeating-request setup. A resource whose acquisition completes after cancellation is released instead of published.
- Camera busy/error, disconnect, and session-configuration failure invalidate once and do not retry. The existing single capture thread still owns one Camera2 source and one AVC encoder; no decoder or audio path was added.
- Stop, disconnect, foreground loss, authenticated-control loss, Forget, and lease expiry converge on invalidation before the existing snapshot-and-clear cleanup. Every cleanup action still runs independently if an earlier platform release throws.
- The Android test artifact now uses `AndroidJUnitRunner` and includes device-runnable checks for authorization-before-cleanup, stale callback rejection across restart, every-throwing cleanup continuation, and duplicate terminal camera failure. These exercise the production policy/gate classes on Android; they deliberately do not pretend to inject real Camera2 driver callbacks.

## Automated and static evidence

- Android `testDebugUnitTest`: 29 JVM tests pass. The 11 new tests exercise generation invalidation, late callbacks, duplicate terminal callbacks, camera busy/error, disconnect, session failure, Stop during partial acquisition, lease-expiry-style cancellation, late MediaCodec-style acquisition, and single-attempt acquisition failure.
- Android `assembleDebug`, `assembleDebugAndroidTest`, `assembleRelease`, `lintDebug`, and `lintRelease`: pass. The instrumentation APK contains four tests but was not run because no device/emulator was attached. Lint reports zero errors and the existing single newer-version warning for the JVM-only `org.json` fixture.
- The debug APK verifies with APK Signature Scheme v2 and one signer. Compiled permissions contain `CAMERA` and no `RECORD_AUDIO`.
- `cargo test --workspace`: 63 tests pass (37 core, 14 daemon, 8 output, 4 preview). Strict Clippy, Rust formatting, QML lint, `git diff --check`, output/preview probes, systemd unit parsing, and Arch metadata parsing pass. The unit parser reports the expected development-host warning for the uninstalled `/usr/bin/omacam-daemon`.
- A fresh constrained-baseline 1280×720@30 H.264 synthetic stream decodes and normalizes successfully. Existing graph tests continue to prove one H.264 decoder, bounded leaky preview/output queues, bounded frames/access units, and no audio path.

## Honest limits

These deterministic JVM tests validate OmaCam's callback decisions and resource ownership, not Android framework or device-driver behavior. The instrumentation suite was compiled only. No instrumented test execution, physical permission dialog, busy camera, real callback timing, camera image, V4L2 consumer, live shell, package installation, or release signature was tested. Those gates remain open.

Exact continuation when an Android 12+ test phone or emulator is attached:

```sh
cd android
JAVA_HOME=/usr/lib/jvm/java-17-openjdk \
ANDROID_HOME=/tmp/omacam-android-sdk \
ANDROID_SDK_ROOT=/tmp/omacam-android-sdk \
./gradlew connectedDebugAndroidTest
```

The four policy tests are necessary but not sufficient for physical qualification. The permission-denial/recovery, camera-busy, real late-callback, resource-release timing, foreground/background, activity-destruction, and recognizable-media procedures still require a physical phone and desktop endpoint.

No firewall, routes, DNS, VPN, USB functions, tethering, hotspot, video devices, Omarchy configuration/processes, or unrelated processes were changed.
