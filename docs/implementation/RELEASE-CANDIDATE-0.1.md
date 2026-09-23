# OmaCam 0.1 release-candidate status

Date: 2026-09-12 (Asia/Kolkata)

## Shippable boundary

The 0.1 candidate uses the Android companion provider. Native UVC remains a
read-only detected option requiring user confirmation. The experimental
browser provider is disabled and reported as unqualified; its session protocol
module is retained for later development but no browser listener, web app, or
WebRTC path is advertised by this candidate.

The release does not provision a kernel module, change networking or firewall
state, enable its user service, install its Omarchy plugin into a live shell, or
select an existing camera automatically.

## Automated evidence from the candidate tree

- `git diff --check`: passed.
- `cargo fmt --all -- --check`: passed after formatting the new browser module.
- `cargo test --workspace --all-targets --locked`: 93 tests passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo build --workspace --release --locked`: passed.
- `qmllint -I /usr/share/omarchy/shell omarchy-plugin/*.qml`: passed.
- Release output CLI and preview dependency probes: passed.
- `packaging/tests/install-lifecycle.sh`: passed.
- `makepkg --force --noconfirm --cleanbuild --dir packaging/arch`: passed and
  produced normal and debug Arch package artifacts.
- `systemd-analyze verify`: the unit parsed; the expected checkout-host warning
  remains because `/usr/bin/omacam-daemon` is not installed on the host.

The repository now has canonical project/license metadata, packages the MIT
license, exposes a single automated verification entry point at
`scripts/verify-release.sh`, and requires an already provisioned character
device in the convenience launcher. The launcher no longer invokes `sudo`,
loads kernel modules, hard-codes UID 1000, or auto-approves pairing.

## Deferred to the owner test pass

Android JVM/APK/lint tasks were not rerun on this date because no valid Android
SDK path is installed or configured in this environment. Existing reports are
historical and are not counted as fresh candidate evidence. The verification
script fails with actionable guidance when `ANDROID_HOME`/`ANDROID_SDK_ROOT` or
JDK 17 is unavailable.

The following remain release gates rather than implementation claims:

1. Fresh Android JVM, lint, debug/release APK, and connected-device tests.
2. Owned `v4l2loopback` provisioning and install/upgrade/uninstall rehearsal.
3. Real Stop, Forget, permission, busy-camera, link-loss, and lease-expiry tests.
4. OBS/browser/Electron consumer, live Omarchy shell, preview-reopen, keyboard,
   scaling, and accessibility tests.
5. Second-OEM, coexistence, 30/60-minute, latency, recovery, and resource trials.
6. Release-owner Android and package signing, source tag/archive checksum, and
   final artifact hash recording.

Do not publish 0.1 until these results are recorded in a dated evidence folder
and the final release checklist is reviewed.
