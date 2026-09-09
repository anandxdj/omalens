# G5-001 — Versioned session-bus snapshot foundation

Status: read-only service boundary implemented and automated/smoke verified without a phone or live shell. Mutating lifecycle ownership, native onboarding, and G5 remain open.

Requirements: R01, R02, R05, R09–R11, R13, R17, R20. Gate: foundation work toward G5; no roadmap gate is complete.

## Objective and implemented result

Add the first real local IPC boundary required by the contracts: a versioned `dev.omacam.Session1` interface on the user session bus at `/dev/omacam/Session`, owned by `dev.omacam.Session`. `GetSnapshot` returns bounded JSON containing schema/API/protocol versions, independent trust/connection/capture/output axes, revision, provider ordering/status, output mode/readiness, preview transport metadata, capability revision, and applied-settings shape. `RunDiagnostics` returns bounded structured JSON containing the existing read-only host/provider reports.

The command-line `omacam-daemon ipc snapshot|diagnostics` client calls those methods and prints only the returned JSON. The panel now consumes `ipc snapshot`, validates both versions, and renders a truthful service-unavailable state if the user service is not running. Preview frames never cross D-Bus and no secret or arbitrary path is accepted. The service remains read-only until it owns the actual control/output lifecycle; no fake Start/Stop success path was added.

## Bounds and safety

- Every D-Bus reply is capped at 65,536 bytes.
- Snapshot data is low-volume JSON; high-rate media remains on the private Unix-FD preview channel.
- The service performs only read-only diagnostics and trust-file inspection.
- No firewall, route, DNS, VPN, USB, tethering, hotspot, device, shell, or unrelated-process changes occur.
- The session bus is protection against other users and accidental misuse, not a boundary against a compromised same-user process.

## Verification performed on 2026-09-09

- `cargo test --workspace`: 62 tests pass (37 core, 13 daemon, 8 output, 4 preview).
- `cargo clippy --workspace --all-targets -- -D warnings`: passes for the new service code.
- `cargo fmt --check`, `git diff --check`, and QML lint pass.
- `dbus-run-session` started the service and successfully called both IPC methods through the thin client.
- Snapshot and diagnostics replies parsed as JSON and remained within the 64 KiB bound.

## Physical, application, and pending status

Physical: not tested. Application/live shell: not tested. The service is not installed or supervised as a user service yet, and panel behavior inside the long-lived shell remains untested.

Pending work is to move the real control/output/session lifecycle under this service, add idempotent intent methods and revisioned events, provide a user-service/package setup path, and complete native onboarding. Start must continue to require phone consent and exact binding; Stop must invalidate before cleanup. G3, G4, G5, G6, G7, the MVP, and the full product remain incomplete.
