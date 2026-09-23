# OmaCam development documentation

Status: proposed product and engineering baseline, prepared 2026-09-08. Implementation evidence and remaining gates are tracked separately under `docs/implementation/`; do not infer completion from this baseline.

OmaCam turns one Android phone into one Linux webcam without changing existing networking or USB services. This repository is named `omalens`; the supplied plan calls the product **OmaCam**. Use OmaCam in these documents, but resolve the public name before reserving package names, application IDs, signing identities, or protocol namespaces.

## Read in this order

| Document | Purpose |
| --- | --- |
| [Original plan](plan.md) | Preserved source vision; not an implementation contract |
| [Plan review](01-plan-review.md) | Flaws, contradictions, feasibility risks, and proposed corrections |
| [PRD](02-prd.md) | Users, release scope, requirements, UX, and success measures |
| [Architecture](03-architecture.md) | Components, ownership, media flow, platform boundaries, and failure isolation |
| [Contracts and states](04-contracts-and-states.md) | Protocol requirements, capabilities, operations, and lifecycle semantics |
| [Security and coexistence](05-security-and-coexistence.md) | Threat model, permission boundaries, privacy, and forbidden actions |
| [Development roadmap](06-development-roadmap.md) | Sequenced work packages and evidence required at each gate |
| [Verification and release](07-verification-and-release.md) | Acceptance matrix, measurement methods, packaging, recovery, and release gates |
| [LLM implementation guide](08-llm-implementation-guide.md) | Working rules, code standards, task format, and handoff instructions |
| [Decisions and sources](09-decisions-and-sources.md) | Proposed decisions, unresolved questions, and verifiable references |
| [Completion plan](implementation/COMPLETION-PLAN.md) | Executable checklist for physical camera, live Omarchy UI, controls, reliability, packaging, and release evidence |

## How to interpret these documents

- **MUST / MUST NOT**: required behavior within the selected release scope.
- **SHOULD**: default recommendation; deviations need a documented reason and equivalent verification.
- **Target**: a proposed measurable engineering objective, not a measured result.
- **Gate**: evidence needed before dependent work can claim readiness.
- **Deferred**: retained product intent, excluded from the proposed first release.
- **Open**: not yet determined; an implementing LLM must not invent supporting evidence.

The PRD is the proposed normative product baseline; contracts and safety documents refine it. The original plan remains unchanged for traceability. Explicit later user decisions supersede these proposals. If two normative documents conflict, resolve the conflict in documentation before implementing the affected behavior.

The companion-first release sequence is a recommendation that narrows the original first-release scope. It does **not** remove the longer-term browser, UVC, scrcpy, audio, and automatic handover vision. Record acceptance or changes to that sequence in the decision register when implementation is commissioned.

## First implementation assignment

Read the LLM guide, then execute **G0: feasibility and release baseline** in the roadmap. Prove the platform and media assumptions before creating a broad framework. This documentation request itself does not authorize implementing code, installing dependencies, editing desktop configuration, or enabling services.
