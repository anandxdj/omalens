# Decisions, open questions, and sources

Prepared 2026-09-08. External sources were checked on this date; branch URLs and documentation can change. Pin exact tested versions/commits during G0. This is a proposal register, not a record of owner approval or implemented behavior.

## Decision register

| ID | Decision | Status | Rationale / revisit condition |
| --- | --- | --- | --- |
| D01 | Use OmaCam as the documentation name; repository remains omalens | Provisional | Original plan's name; owner must settle public branding and identifiers before distribution |
| D02 | Native companion plus existing local IP network for MVP | Proposed scope change | Reduces backend/consent/security matrix; retain full hybrid vision in G8 |
| D03 | Modular Rust daemon, thin QML, Kotlin companion | Recommended | Matches original technology direction and separates desktop presentation from camera processing |
| D04 | Dedicated output writer independent of media-worker failure | Recommended; G0 proof required | Improves continuity without placing native decode in the shell; measure process/IPC/idle costs |
| D05 | V4L2 output first; no second separately advertised PipeWire camera by default | Recommended | Avoid duplicate sources; actual portal compatibility still needs qualification |
| D06 | Benchmark maintained WebRTC implementation first | Provisional | Prefer existing media/congestion/security machinery; exact library and codec depend on evidence |
| D07 | Ask default; Auto-connect authenticates only; explicit phone capture consent | Proposed normative policy | Resolves privacy/lifecycle ambiguity and preserves phone control |
| D08 | Neutral output on disarm/loss, fixed output mode while writer is healthy | Recommended | Prevent stale private imagery and consumer renegotiation during adaptation |
| D09 | Separate system package setup from shell plugin installation | Evidence-backed constraint | Documented Omarchy installer cannot install the privileged prerequisites itself |
| D10 | No automatic transport/provider handover in MVP | Proposed sequencing | Prove single-path reliability first; later use bounded gaps and rollback |
| D11 | No phone microphone in MVP | Proposed sequencing | Avoid permissions, virtual audio routing, and clock-drift scope until video is qualified |
| D12 | Stop capture on desktop lock/suspend/logout; ten-second unreachable-phone lease | Proposed privacy default | Prevent unintended continuation; requires session-event integration and lifecycle tests |

## Open decisions and when they must be resolved

| ID | Question | Resolution needed | Safe interim handling |
| --- | --- | --- | --- |
| O01 | Public product name, package IDs, Android application ID, signing ownership? | G0 before durable identifiers/distribution | Use working documentation name; reserve nothing |
| O02 | Accept companion-first MVP or require browser-first first release? | G0 before committing provider implementation | Use proposal for planning only; browser-first requires moving secure-context spike into critical path |
| O03 | Which Omarchy release, kernels, GPUs, Android models/versions and consumer packages are supported? | G0, then G7 qualification | No universal compatibility claim |
| O04 | Which maintained transport/codec libraries interoperate on target devices? | G0/G1 | Benchmark candidates; no custom crypto or speculative H.265/1080p60 promise |
| O05 | How does preview reach the actual shell safely and efficiently? | G0 before UI/media integration | Prove bounded mechanism; no large D-Bus frame payloads |
| O06 | Which Linux secret store and exact pairing protocol/library are available? | G1 before pairing code | Fail safe when unavailable; no accept-all TLS or plaintext substitute |
| O07 | What GUI/package/distribution workflow supplies dependencies and signed companion builds? | G0 design; G7 validation | State separate prerequisites; do not promise plugin-only installation |
| O08 | What streaming CPU/GPU, quality, and thermal budgets fit the reference rig? | G0 before adaptation tuning | PRD targets remain proposed; no invented benchmark numbers |
| O09 | Is multi-user desktop/device access part of first release? | G0/G2 before permissions design | Qualify one active desktop user; no world-writable device rules |
| O10 | Which provider and feature is the first expansion? | G8 after MVP evidence | Preserve simple extension points without implementing placeholders |

## Source notes

The original plan contains opaque chat citations. They are preserved in that source document, but new decisions should cite these readable references or updated primary sources. Facts and product inferences are distinguished below.

### S1 Omarchy shell

[Official shell README, quattro branch](https://github.com/omacom/omarchy/blob/quattro/shell/README.md) and [official shell-plugin manual](https://omarchy.org/manual/shell-plugins/).

Verified: the documented shell hosts QML plugins, and its plugin add workflow does not execute sudo or installation hooks. Inference for OmaCam: system dependencies and privileged camera setup need a separate workflow. This source does not establish which Omarchy release the user's machine runs; G0 must inspect it. Avoid treating a moving branch as a stable API version.

### S2 V4L2loopback

[Upstream repository and documentation](https://github.com/v4l2loopback/v4l2loopback).

Verified: `exclusive_caps` advertises output-only initially and capture capability with a producer attached; upstream documents labels, timeout behavior, and dynamic device management. Inference for OmaCam: a continuously attached writer and safe frames should improve camera availability through transport loss. Actual consumer behavior and writer-crash behavior need tests; no source proves universal persistence.

### S3 Browser capture

[W3C Media Capture and Streams](https://www.w3.org/TR/mediacapture-streams/).

Verified: camera capture interfaces are restricted to secure contexts and permission-controlled access. Inference for OmaCam: an ordinary LAN HTTP QR page is not a sufficient browser onboarding architecture. Resolve trusted delivery, signaling, and peer identity before promising zero-install capture. A native QR certificate/key binding does not automatically establish browser origin trust.

### S4 Android cameras

[Android multi-camera API guidance](https://developer.android.com/media/camera/camera2/multi-camera).

Verified: logical and physical camera relationships and supported combinations depend on hardware/OEM behavior; independent physical-camera opening is not a universal guarantee. Inference for OmaCam: report usable modes and conditional controls, not assumed ID-to-lens mappings or a Cartesian product of size/FPS lists.

### S5 Android lifecycle

[Android foreground-service background-start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start).

Verified: camera/microphone while-in-use permissions constrain starting foreground services from the background. Inference for OmaCam: a wake lock, trusted pairing, or desktop request cannot substitute for a compliant phone lifecycle and consent flow. Recheck rules against the selected target SDK and test OEM behavior.

### S6 scrcpy

[Upstream camera documentation](https://github.com/Genymobile/scrcpy/blob/master/doc/camera.md).

Verified: camera mirroring requires Android 12 or later in the inspected documentation; camera mode defaults its audio source to the microphone. Inference for OmaCam: a future adapter must explicitly disable audio and qualify the pinned version. Do not assume all desired runtime controls exist in an arbitrary installed version or that ADB authorization equals OmaCam pairing.

### S7 Camera portal

[XDG Desktop Portal Camera interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Camera.html).

Verified: the camera portal mediates camera access and exposes a PipeWire remote for permitted camera access. Inference for OmaCam: app packaging, portal backend, and session media configuration must be tested. Merely setting node properties is not proof that a sandboxed application sees the virtual camera.

### S8 Android native webcam

[AOSP: use a device as a webcam](https://source.android.com/docs/core/camera/webcam).

Verified: DeviceAsWebcam support begins with Android 14 QPR1-capable implementations and requires device enablement; its documented flow includes user USB-function selection. Inference for OmaCam: detect already-enabled UVC as a future source; do not infer availability from Android version alone or change USB functions to obtain it.

## Updating a decision

Add the date, decision owner or evidence author, status, chosen option, alternatives, measured results, consequences, and superseded decision ID. Update every affected requirement, contract, test, and roadmap gate. Do not erase unresolved risks by changing “proposed” to “accepted” without a real decision or test result.
