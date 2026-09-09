# G5-002 — Service-owned lifecycle intents and revisioned state

Status: implemented and automated/synthetic verified on 2026-09-09. Physical phone, live-shell, real V4L2 application, installation, release packaging, and signing evidence remain open. This does not complete G2, G3, G4, G5, the MVP, or the product.

Requirements: R01, R02, R05, R09–R11, R13, R17–R20. Gates: foundation toward G2/G5; no gate is complete.

## Implemented

- `omacam-daemon service` now creates the authoritative long-lived runtime used by the authenticated control/capture path and the `dev.omacam.Session1` user-session D-Bus object. Snapshots no longer construct a fresh policy per request.
- The bounded schema-2 snapshot contains monotonic revision, independent policy axes, provider/output/preview truth, the latest 32 operations, typed terminal errors, and no media or secrets.
- `RequestStart`, `Stop`, `ForgetPeer`, and `RequestDiagnostics` accept caller operation IDs of at most 64 bytes. Reuse for the same intent is idempotent; reuse for another intent is rejected. The intent queue is bounded to 16.
- A successful method reply means only `accepted`. Start remains pending through desktop request and phone consent and succeeds only after the exact request correlation produces the exact peer/control-connection/media-session/generation binding and the first authenticated frame starts that generation. Decline and startup/output failure become typed operation failures.
- Stop and Forget invalidate authorization/generation and publish neutral/idle state before resource/trust cleanup. Repeated Stop is safe.
- `StateChanged(revision)` is emitted for low-volume state/operation changes. `ipc events` is a bounded-data watcher. The panel treats events as refresh hints, fetches full snapshots, and falls back to periodic full refresh if the event stream fails.
- The panel enables Start only for trusted + online + idle state, enables Stop only outside idle, shows phone-consent/streaming/stopped truth, and surfaces service or operation errors. It still launches only the isolated RGBx/JPEG preview consumer; H.264 and network media never enter QML.
- A prototype systemd user unit and Arch `PKGBUILD` install only fixed binaries, documentation, plugin files, and the user unit. There is no root daemon, install hook, arbitrary script execution, automatic enable/start, or automatic network/device configuration. The user must intentionally provide an existing listen address and separately provisioned OmaCam-owned V4L2 node.

## Automated and static evidence

- `cargo test --workspace`: 63 pass (37 core, 14 daemon, 8 output, 4 preview).
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `git diff --check`: pass.
- Android `testDebugUnitTest`, `assembleDebug`, and `lintDebug`: pass with 18 JVM tests, zero lint errors, and one dependency-version warning.
- APK static inspection: v2 debug signature verifies with one signer; declared permissions contain CAMERA, INTERNET, multicast, and network state, but no RECORD_AUDIO.
- `qmllint omarchy-plugin/*.qml`: passes.
- `systemd-analyze verify packaging/systemd/omacam.service` parses the unit; it reports the expected prototype-host warning that `/usr/bin/omacam-daemon` is not installed.
- `makepkg --printsrcinfo` parses the prototype package metadata.

## Synthetic evidence

- A private `dbus-run-session` smoke started the unpaired service on the current existing interface without changing it, parsed bounded schema-2 snapshots, accepted and completed diagnostics and repeated Stop with the same operation ID, and returned typed `peer_not_trusted` for Start. The owned process was stopped afterward.
- `omacam-output --probe` reports neutral, H.264 ingress, and shared-preview dependencies ready. `omacam-preview --probe` reports its isolated consumer dependencies ready.
- A fresh constrained-baseline 1280×720@30 synthetic H.264 stream passed the bounded OpenH264 decode, conversion, rate normalization, and fixed YUY2 caps into `fakesink`. An initial ad-hoc compositor command was invalid because its standalone filter did not fully specify output caps; it was discarded as a harness error rather than recorded as product evidence.
- Existing Rust media tests continue to prove exact binding, bounded framing/queues, post-Stop rejection, and one-decoder topology. This is synthetic evidence, not a physical camera claim.

## Physical, live-shell, application, packaging, signing, and pending status

- Physical: not run; the phone was unavailable. G3 restart/spoof/revocation and G4 consent/permission/busy/Stop/loss/lease/resource trials remain open.
- Live shell: not run and the Omarchy shell was not started or restarted. Preview close/reopen and shell stability remain open.
- Application: no real V4L2 consumer or recognizable phone image was used. Neutral-within-500-ms, handle persistence, and 30/60-minute qualifications remain open.
- Packaging: prototype metadata/unit only. No package was built, installed, enabled, upgraded, rolled back, or uninstalled; output-device provisioning and ownership receipts remain unresolved.
- Signing: only the debug APK signature was checked. Release identity, provenance, licensing, public names/IDs, and release signing remain unresolved.
- Pending: callback-level Camera2/MediaCodec fault injection is the next hardware-independent package. The service still requires an intentionally supplied existing address/output node and does not implement onboarding or privileged provisioning.

No firewall, routes, DNS, VPN, USB functions, tethering, hotspot, shared video devices, unrelated processes, or live-shell state were modified.
