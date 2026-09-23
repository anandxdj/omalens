# G5-004 — Service-lifetime output integration

Status: automated integration passed; physical, application, live-shell, and
release gates remain open.

On 2026-09-09 the output worker was moved above individual authenticated
control connections. One long-lived raw writer owns the output/preview
pipeline for the service lifetime and emits neutral I420 when no current frame
is eligible. A bounded reset/rebind protocol separates that writer from a
single decoder belonging to the active binding. Reset stops and reaps the
decoder, clears the two-frame freshness-biased queue, and rejects stale or
out-of-order bind/control/media input. Stop and diagnostics intents remain
available without a trusted peer, and output-child failure is reflected as a
service failure rather than success.

Coordinator verification:

- `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo build --release --locked --workspace` passed.
- `cargo test --workspace --all-targets` passed 83 tests: 39 core, 23 daemon,
  17 output, and 4 preview. The output regressions cover reset/rebind, stale
  generations/control bytes, neutral persistence, one decoder, bounded input
  and raw queues, and service/raw-pipeline separation.
- Both release probes passed. A synthetic constrained-baseline Annex-B
  1280x720@30 stream encoded, parsed, decoded, converted, rate-normalized to
  YUY2, and reached `fakesink`.
- A private session-bus smoke produced 10 revision events and verified bounded
  schema-2 snapshot, diagnostics, repeat-Stop idempotency, typed
  `peer_not_trusted` Start rejection, and truthful `output_unavailable` state
  when `/dev/null` was deliberately rejected as an output device.
- `qmllint -I /usr/share/omarchy/shell omarchy-plugin/*.qml` passed. The panel
  now bounds snapshots/operations/preview payloads, recovers from lost or
  stale revisions, requires output readiness for Start, and exposes keyboard
  and accessibility actions. No live Omarchy configuration or shell changed.
- Android JDK-17 verification passed 32 JVM tests and all debug, instrumentation
  APK, release, and lint tasks (126 Gradle actions). Debug v2 signing and the
  CAMERA/INTERNET/multicast/network-state permission set passed inspection;
  RECORD_AUDIO is absent. No attached device meant connected tests were not
  run.
- A clean `makepkg` build, metadata/archive inspection, corrected
  `gst-plugins-bad` ownership check, and the 11-assertion isolated lifecycle
  harness passed. Exact artifacts are in `packaging/verification-report.md`.
- Coordinator review traced authorization invalidation before cleanup, output
  ownership above connection acceptance, decoder teardown before rebind,
  bounded channel/allocation limits, generation checks before forwarding,
  child-health publication, preview payload/lifecycle bounds, UI operation
  gating, and local Stop/Forget behavior. No additional automated race, leak,
  stale-generation, security-boundary, or UI-truth defect was found; this code
  review is not a substitute for the open physical and live-shell evidence.

Evidence limits are explicit. No OmaCam-owned V4L2 device was available; the
host `/dev/video0` and `/dev/video1` nodes were only identified and never
opened. Therefore consumer-handle persistence, recognizable phone imagery,
Stop-to-neutral timing, application behavior, and sustained operation remain
unqualified. No Android device/emulator was attached, so instrumented and
physical permission, busy-camera, Camera2, codec, revocation, and restart paths
remain open. No controlled live-shell run was authorized. Rootful package
install/upgrade/rollback/uninstall, real release identity/provenance/license,
and signing also remain open. Staged-root systemd verification passes with
`SYSTEMD_UNIT_PATH=/usr/lib/systemd/user`, `--root` set to the generated package
root, and the root-relative user-unit path.

The coordinated workers could not send their required Orca `worker_done`
messages because their worker sandboxes could not reach the coordinator's Orca
runtime; coordinator-side commands remained functional. Their terminal
transcripts and exact reports were inspected, and every claimed automated
result above was rerun by the coordinator. The coordinator used Orca's explicit
`worker-abandon` recovery without claiming any process stopped, marked all four
bounded tasks completed with their blockers, and requested release. Orca
retained each with `retainedReason: external_terminal` and performed no process
action, so no unowned process was closed. This is an orchestration
transport/lifecycle limitation, not product evidence.
