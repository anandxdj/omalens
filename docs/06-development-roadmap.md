# Development roadmap and gates

This is an execution order for future implementation, not authorization to write code during the present documentation task. Every gate has observable outputs. Do not mark a gate complete from mocks, generated reports, or assumed hardware behavior.

## G0 — Establish feasibility and the release baseline

Dependencies: read all baseline documents.

Work packages:

1. Record actual Omarchy version/commit, plugin API, Quickshell/Qt versions, kernel, Secure Boot status, graphics stack, portal implementation, and existing loopback usage using read-only inspection.
2. Record available Android models and OS versions. Decide minimum SDK, target SDK, and tested support matrix from actual API needs and distribution requirements.
3. Resolve working name and proposed companion-first release scope. Reserve no public IDs before that decision.
4. During authorized implementation, build disposable feasibility spikes: native camera capture to a maintained media stack; local authenticated media path; stable V4L2 output with neutral frames; preview delivery into the actual shell.
5. Measure baseline CPU, memory, copying cost, optical latency, and first-frame time. Test existing USB tethering and hotspot continuity without reconfiguration.
6. Determine package installation privileges and a supported user-facing dependency installation path. Validate terminal-free onboarding instead of assuming the plugin installer provides it.

Deliverables: environment matrix, spike results with reproduction steps, selected libraries/versions/licenses, resource budgets, installation design, updated decision register. Keep throwaway spikes outside final architecture unless explicitly adopted after review.

Exit: at least one real phone can feed a qualified Linux consumer through the proposed media/output path, the preview mechanism is proven, and installer limitations are understood. Security shortcuts in isolated synthetic-media experiments cannot become product defaults. If a spike cannot be run, mark its dependent decision blocked and continue only independent design work.

## G1 — Freeze contracts and security design

Dependencies: G0 platform and transport decisions.

Work: create exact D-Bus and wire schemas; define session generations, limits, cancellation, capability tuples, typed errors, key storage, pairing cryptography, media identity binding, migration strategy, and protocol version policy. Review the threat model against each concrete dependency.

Deliverables: contract specifications and fixtures, state transition cases, security design record, traceability from R03–R08/R13/R17/R20 to tests.

Exit: no unspecified bootstrap authentication, session-consent semantics, or unbounded input/resource path. Conflicting messages and replay behavior have expected outcomes before coding.

## G2 — Establish the output and process foundation

Dependencies: G0 output proof; G1 local contracts.

Work: implement service lifecycle, owned-device discovery, output writer, neutral frames, bounded media/preview interfaces, read-only diagnostics, and minimal storage migrations. Use synthetic frames to exercise lifecycle without phone permissions.

Deliverables: one labeled camera, readiness state, crash/restart behavior, scoped package prototype, meaningful ownership and queue tests.

Exit: Chrome/Chromium, Firefox, and OBS on the qualification rig read output; the writer survives a media-worker failure and preserves the active consumer handle. Writer failure is reported honestly. Another loopback user's device remains untouched.

## G3 — Secure pairing and session control

Dependencies: G1; G2 state/readiness reporting.

Work: companion scanner and trust storage, invitation lifecycle, peer approval, authenticated control, Stop/Forget, session lease, local discovery hints, version rejection, and Ask/Auto-connect/Ignore semantics.

Deliverables: end-to-end pairing without camera streaming, attack fixtures, restart and revocation tests.

Exit: expiry, replay, concurrent redemption, wrong key, changed identity, forged discovery, malformed invitation, and unsupported-version tests pass. No Start occurs merely from trust or reconnect.

## G4 — Deliver one complete camera path

Dependencies: G2 and G3.

Work: explicit phone Start flow, Camera2/MediaCodec lifecycle, valid mode discovery, local media transport, shared decode, rotation, stable output normalization, and first preview. Default to a qualified 720p30 configuration.

Deliverables: phone-to-consumer video, source/output status, Stop on both devices, lease expiry, busy-camera and denied-permission recovery.

Exit: a real meeting/capture application receives fresh frames; Stop clears imagery and releases resources; no audio is captured; 30-minute trial succeeds before adding controls.

## G5 — Add product UI and supported controls

Dependencies: G4.

Work: bar widget and independent panel entry, onboarding/readiness screens, camera selection, supported zoom/exposure compensation/torch, black-screen mode, preview mirroring preference, keyboard/accessibility behavior, applied-value feedback.

Deliverables: user flows tied to PRD requirements and real capability fixtures for differing phones.

Exit: user completes pairing and a call without technical identifiers; unsupported controls are hidden; switching failure is recoverable; hiding/reopening the widget does not duplicate capture or stop an intended session.

## G6 — Reliability, adaptation, and coexistence

Dependencies: G4; G5 for user-visible failures.

Work: bounded recovery and quality adaptation, sustained thermal behavior, worker fault injection, queue pressure, multiple consumers, interface churn, lock/suspend handling, and stale-image tests. Add hysteresis: downgrade on sustained degradation and require a stable recovery interval before upgrading; record tuned thresholds with evidence.

Deliverables: repeated benchmark results, state/fault traces without secrets, and USB/hotspot continuity evidence.

Exit: all safety tests pass, the 60-minute qualification session completes, and performance meets the chosen release budget or a documented scope decision resolves the gap. No automatic provider handover in this milestone.

## G7 — Installation, upgrades, and first release

Dependencies: G0–G6.

Work: reproducible packages and companion build, signing/provenance, user-facing prerequisite setup, owned-file receipts, kernel upgrade handling, partial-install recovery, protocol compatibility checks, rollback, safe uninstall, diagnostics export, and support documentation.

Deliverables: exact compatibility matrix, release checklist results, known limitations, install/uninstall instructions, and rollback rehearsal.

Exit: every MVP requirement has passing evidence; failures and unsupported environments are disclosed. Do not advertise the original full-vision feature list as shipped.

## G8 — Expand one capability at a time

Dependencies: released and measured MVP. Each expansion repeats contracts, security, coexistence, and acceptance work for its own scope.

| Expansion | Required proof before release |
| --- | --- |
| Existing USB-tether IP route | Reachable authenticated path, sustained internet continuity, cable removal recovery, no USB mutation |
| Native UVC | Explicit already-enabled source selection, device ownership, supported formats/controls, stable output; no assumed identity equivalence |
| scrcpy | Pinned supported version, Android support boundary, existing ADB authorization, audio explicitly disabled, own-process cleanup, no shared ADB reset |
| Physical lenses/manual controls | Valid session modes per device, friendly naming confidence, control dependencies and switching rollback |
| Browser/PWA | Trusted secure context, authenticated signaling bootstrap, generic QR/app onboarding story, permission and background limits, no TLS bypass |
| Phone microphone | Explicit opt-in, PipeWire source ownership, no default routing changes, timestamp/drift synchronization, silence and revocation tests |
| Automatic handover | Known-safe candidate, stable output mode, hysteresis, bounded gap, identity continuity, rollback, no concurrent camera assumption |

## Work package template

For every assigned task record: objective; gate and requirement IDs; preconditions; authorized paths; expected artifacts; relevant contracts; safety invariants; test cases and required hardware; out-of-scope work; rollback; completion evidence; unresolved blockers.

Example assignment: “G3 / R03, R17: implement one-time invitation redemption and local revocation after the protocol design is approved. Test expired, reused, simultaneous, cancelled, and changed-key attempts. Do not add media capture or network reconfiguration. Completion requires passing security fixtures and a real two-device pairing/forget trial.”
