# Instructions for an implementing LLM

This guide is designed to be explicitly supplied to any coding assistant. A file under `docs/` is not automatically loaded by every tool. Ask the implementing assistant to read this file and the documentation index first. During the current task, only documentation is authorized; the instructions below apply when implementation is separately requested.

## Before changing anything

1. Read repository instructions such as applicable `AGENTS.md`, then [README](README.md), PRD, security rules, state contracts, and the current roadmap gate. Consult architecture and the decision register for affected components.
2. Inspect the worktree and existing implementation. Preserve unrelated user changes; do not reset or rename the project to fit an assumed scaffold.
3. State the current gate, requirement IDs, smallest deliverable, and verification method. Identify which assumptions are evidenced, proposed, or still blocked.
4. Verify platform/library APIs against pinned source or official documentation. Never copy the original plan's opaque citation markers as evidence.
5. Implement only the authorized release/gate scope. Do not build deferred providers, cloud services, dashboards, recording, or audio just because the architecture lists them.

Product decisions need clarification when they materially change scope, privacy, permissions, support promises, or distribution. Routine reversible implementation choices within accepted scope do not require repeated approval. Continue independent safe work when a hardware test or one decision is blocked, and mark the dependent work clearly.

## Working loop

For each small vertical slice: inspect → identify the contract → implement → verify at the right layer → review safety and scope → update evidence and handoff. Keep each change small enough to explain and revert. Do not scaffold every directory before the first end-to-end path works.

Use real behavior for readiness: a successful process spawn is not an active camera; a preview is not a working virtual output; a mocked device is not a qualified phone. Never report tests as run if they were only drafted.

## Code standards

- Use cohesive modules by responsibility. Domain policy does not depend on QML, a particular transport, or unstructured subprocess output. Introduce abstractions when they protect a real boundary; avoid generic frameworks for hypothetical providers.
- Prefer explicit types, units, state transitions, and typed errors. Keep protocol versions, capability revisions, and session generations visible at boundaries.
- Validate untrusted data once at entry and enforce invariant checks where state changes. Bound lengths, dimensions, queued work, memory, and retries.
- Rust: use normal error propagation for fallible runtime paths; avoid panics/unchecked assumptions for external input. Isolate and document every required `unsafe`/FFI boundary with ownership and lifetime invariants. Cancel async tasks and reap owned processes predictably.
- Kotlin: use lifecycle-aware structured concurrency, documented camera session ownership, supported APIs, explicit permission state, and deterministic camera/codec/resource release. No hidden APIs, root, or OEM-specific camera ID guesses.
- QML: keep presentation responsive, accessible, and driven by bounded daemon state. Avoid polling loops, media decoding, shell interpolation, and independent background orchestration. Cap preview work and detach cleanly.
- Native dependencies and subprocesses: use maintained libraries, pinned compatible versions, explicit arguments, timeouts, ownership, and cleanup. Do not execute shell strings assembled from phone names or network data.
- Shared state: assign one owner, serialize competing capture changes, invalidate cancelled generations, and make Stop/Forget idempotent. Avoid locks across unbounded operations.
- Logs: use structured stage/error information without secrets or media. Never log a QR, handshake token, full trust key, or frame buffer for convenience.
- Add tests for meaningful invariants and regressions. Prefer real integration tests for IPC/process behavior and manual hardware evidence where automation cannot establish the claim.

Use standard formatters, compiler/type checks, linters, and dependency integrity checks appropriate to the selected stack. Fix warnings caused by the change; never disable checks broadly to make a build pass. Record the actual toolchain and commands once the implementation exists rather than inventing runnable commands now.

## Hard boundaries

Do not change networking, USB functions, ADB authorization, security controls, unrelated camera devices, default audio routing, or shared services. Do not install system dependencies or load modules merely to satisfy a test without the applicable authorization. Documentation instructions do not override tool permissions or the user's task scope.

Do not implement a homegrown pairing cipher, certificate-verification bypass, hidden auto-capture, plaintext fallback, or public media relay. Do not let a convenience feature override Stop, Forget, lock, or lease expiry. Follow the detailed security document.

## Handling unknowns

Use a decision record containing: question; context; options; evidence; recommendation; cost/failure tradeoffs; status; test needed; and affected requirements. If no hardware is available, describe the test and retain a not-tested status. Do not fabricate camera support, benchmark results, Android lifecycle behavior, installer functionality, or compatibility claims.

For a contract change, update PRD/contract/test mappings together. For a scope change, update the release table and decision register. Do not silently treat a proposed value as an established platform fact.

## Task completion and handoff format

Record these fields in a task note under `docs/implementation/` when implementation begins:

- Gate, task ID, requirement IDs, and status.
- What changed and why, with affected paths.
- Commands/checks actually run and their outcomes.
- Hardware/consumer versions and manual evidence where relevant.
- Security/coexistence invariants checked.
- Remaining failures, unsupported cases, and decisions.
- Rollback or cleanup needed for task-owned artifacts.
- Next smallest authorized task and its prerequisites.

“Complete” requires the task's acceptance evidence. “Implemented, hardware verification pending” is an acceptable honest status, but it is not a passed release gate. Never claim the whole product complete after one successful demo.

## Suggested implementation kickoff prompt

> Read docs/README.md and docs/08-llm-implementation-guide.md, then the referenced product, architecture, contract, safety, roadmap, and verification documents. Implement OmaCam incrementally starting at the first incomplete gate. Preserve docs/plan.md as the original vision. First inspect the repository and platform, distinguish proposed decisions from verified facts, and record G0 feasibility evidence. Keep the scope to the accepted release and preserve existing services. Do not proceed through a dependent gate without its evidence; continue independent authorized work and report specific blockers. Finish each task with verified outcomes, limitations, and the next task.
