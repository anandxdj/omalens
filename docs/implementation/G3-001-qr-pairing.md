# G3-001 — Secure QR pairing vertical slice

Status: implementation and desktop smoke checks complete; real-phone verification blocked until an Android device is visible to ADB.

Requirements: R03, R04, R05, R17. Gate: partial G3 only. This slice establishes trust and deliberately does not start camera capture or media transport.

## What is implemented

The desktop command creates a single 120-second invitation, renders it in the terminal and as a mode-0600 SVG, and listens on one explicitly selected private or link-local address. The QR contains a random 128-bit session identifier, a random 256-bit one-use token, a random 256-bit challenge, the concrete endpoint, expiry hint, desktop display name, and SHA-256 binding to the desktop certificate. The token and challenge are redacted from Rust debug output.

The Android companion scans only QR codes through Google Play services, rejects unknown/oversized/version-mismatched input before connecting, accepts only numeric private or link-local endpoints, generates a P-256 identity key in Android Keystore, and signs a length-delimited canonical transcript. It connects with TLS 1.3 and accepts only the exact certificate fingerprint carried by the scanned QR. There is no accept-all trust manager, CA bypass fallback, cleartext mode, arbitrary URL opening, microphone access, or media start.

The phone displays a six-digit short authentication string before its explicit approval. After a valid signed claim, the desktop displays the independently computed string and requires the user to type it. A trust record is committed only after both approvals. The desktop refuses silent replacement of an existing phone. Expiry, cancellation, rejection, or a partial ceremony does not create trust. The credential-bearing SVG is deleted on success, expiry, rejection, and handled Ctrl-C cancellation.

The persistent desktop identity is a self-signed certificate/key stored in an application-owned mode-0600 file. The paired phone record contains its public identity only and is also mode 0600. A future packaging task must evaluate Secret Service integration and migration before distribution. Android signing is debug-only; no release identity has been created.

## Build evidence from 2026-09-08

- `cargo test --workspace`: 26 tests pass after this slice (20 core, 4 daemon, 2 output).
- `cargo clippy --workspace --all-targets -- -D warnings`: passes.
- Android `assembleDebug`, `lintDebug`, and `testDebugUnitTest`: build succeeds; lint reports no issues; no JVM unit-test source exists yet.
- Debug APK signature verification passes with APK Signature Scheme v2. The artifact is `android/app/build/outputs/apk/debug/app-debug.apk`.
- A live desktop listener bound to the current private Wi-Fi address and produced a mode-0600 QR/identity. The no-client invitation expired after 120 seconds and removed its QR. A second run was cancelled with Ctrl-C and also removed its QR.
- `/tmp/omacam-android-sdk/platform-tools/adb devices -l` returned no devices. Therefore scanning, Kotlin/Rust transcript interoperability, TLS pinning on Android, the two-screen code, trust commit across both devices, and OEM behavior are **not yet physically verified**.

## First real-phone test

Prerequisites: Android 12 or newer with Google Play services, phone and laptop on a mutually reachable private network, and USB debugging only for installing this development APK. OmaCam itself does not use ADB for pairing and does not change USB or network settings.

1. On the OnePlus, enable Developer options and USB debugging. Reconnect USB, select a normal data-capable mode if the phone asks, and accept the computer's RSA authorization prompt. Do not disable tethering, hotspot, VPN, or security settings for this test.

2. Confirm the phone appears as `device`, not blank or `unauthorized`:

   ```bash
   /tmp/omacam-android-sdk/platform-tools/adb devices -l
   ```

3. Build and install the debug companion:

   ```bash
   cd /home/anandxdj/Projects/omalens/android
   JAVA_HOME=/usr/lib/jvm/java-17-openjdk ANDROID_SDK_ROOT=/tmp/omacam-android-sdk ./gradlew --no-daemon assembleDebug lintDebug
   /tmp/omacam-android-sdk/platform-tools/adb install -r app/build/outputs/apk/debug/app-debug.apk
   ```

4. Find the laptop address reachable from the phone:

   ```bash
   ip -4 -brief address show up
   ```

   During implementation, Wi-Fi was `10.87.237.216/24` and a likely USB-tether interface was `10.148.133.55/24`; addresses can change. Prefer the address on the same Wi-Fi as the phone. Do not use `127.0.0.1`, a public address, or an address the phone cannot route to.

5. From the repository root, start one invitation, replacing the example address if it changed:

   ```bash
   cargo run -p omacam-daemon -- pair serve --endpoint 10.87.237.216:47123
   ```

6. Open OmaCam on the phone, tap **Scan desktop QR**, and scan the terminal QR. Check the laptop name and six-digit code on the phone. Tap **Approve pairing** only if they are expected.

7. The desktop now prints the phone name and its own six-digit code. If it exactly matches, type those six digits and press Enter. Any other input rejects the ceremony. Success must say that camera access was not started.

8. Verify the local trust result:

   ```bash
   cargo run -p omacam-daemon -- pair status
   ```

To repeat from a clean desktop trust state, explicitly run:

```bash
cargo run -p omacam-daemon -- pair forget
```

This removes only the paired-phone public trust record and retains the desktop identity. Reopening the current development app permits another scan. Clearing app storage or uninstalling the app removes its local trust metadata; Android Keystore cleanup behavior must be recorded during the device trial.

## Required negative checks before closing G3-001

- Let a QR sit for 120 seconds: the desktop must report expiry, remove the SVG, and accept no later scan.
- Press Ctrl-C while waiting: the desktop must cancel and remove the SVG.
- Scan a screenshot from an expired run: no trust record may appear.
- Change one QR field in a fixture: strict parsing, certificate pinning, token verification, or signature binding must reject it.
- Send the same valid claim twice or race two clients: only the first valid claimant may reach approval; no second trust commit is allowed.
- Reject or enter a different code on either screen: neither side may show paired.
- Pair successfully, run `pair status`, restart both sides, and verify stored identities before implementing auto-connect.
- Confirm Android's permission page shows no microphone permission and no camera permission owned by OmaCam for this scanner-only build.
- Repeat while USB tethering is carrying traffic and confirm pairing causes no USB-function, route, DNS, VPN, or interface mutation.

Do not claim this gate complete until the real-device checks are recorded. Camera preview and streaming belong to later G4/G5 work and must remain impossible from scanning alone.
