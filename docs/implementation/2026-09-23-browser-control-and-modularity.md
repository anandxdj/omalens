# Browser control and modularity execution plan

Status: in progress. This plan tracks implementation and evidence; an unchecked
hardware or latency item is not a pass.

## Baseline and boundaries

- Preserve the uncommitted 2026-09-12 browser-first work. The recoverable copy is
  `/data/user-data/Projects/omacam-baseline-QRjmB1/` (tracked patch and untracked
  archive). Do not commit or discard pre-existing edits as part of refactoring.
- Use the [OpenAssets backend](https://github.com/anandxdj/open_assets) as a
  structural reference: place request handling, state transitions, protocol
  contracts, capture, and output pipeline construction in modules with explicit
  responsibilities. Do not transplant its TypeScript framework into Rust.
- Keep the native control listener at 47123 and browser signaling at 8443.
  Keep browser ICE host-only and avoid external signaling or relay services.
- Phone Stop, permission loss, or session expiry must invalidate control and
  clear output. A stopped session never restarts automatically.

## Work packages and acceptance

1. Browser Rust: capability and applied-state contract, authorized phone/desktop
   session, acknowledged command relay, terminal Stop, heartbeat expiry, shared
   decoded preview, bounded queues, and failure tests. Split HTTP, session, RTC,
   and preview responsibilities into focused modules.
2. Browser UI: separate desktop control room and phone capture roles, Omarchy
   styling, supported controls, applied-value display, accessible dim/wake-lock
   lifecycle, responsive layout, and browser behavior checks. Organize markup,
   styles, and role-specific scripts separately.
3. Native companion: enumerate actual Camera2 capabilities and add acknowledged
   controls that can be applied safely. Split protocol, camera, and activity
   responsibilities only where the resulting module owns a coherent concern.
4. Output: selectable PipeWire source backend, V4L2 compatibility backend,
   single decoder and bounded preview branch. Split CLI, pipeline, decode/mux,
   and tests by ownership. Keep neutral output after Stop.
5. Integration: align JSON contracts and lifecycle across both providers, run
   formatting, Rust tests, clippy, Android JVM tests/build, and browser checks.
   Fix interface failures before broad refactoring.

## Physical qualification required before release labels

- Confirm native and browser paths on a phone through an owned virtual-camera
  device and representative consumers. ADB currently reports no connected
  device; automated builds cannot stand in for this evidence.
- Test disconnect, congestion, permission revoke, torch/zoom/exposure support,
  thermal load, Stop, dim exit, stale-frame clearing, and PipeWire compatibility
  in OBS, Chromium, Firefox, Electron, and relevant sandboxed applications.
- Record at least 100 optical transitions on 5 GHz for each provider, reporting
  median, p95, maximum, frame drops, and reconnect time. Show a "Fastest
  wireless" label only after a measured target is met.
