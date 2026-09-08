# Verification, packaging, and release

Status: test plan only. No scenario below has been executed for this project.

## Evidence rules

Use the smallest test layer that can establish the claim. Unit tests prove policy and validation; integration tests prove process and protocol behavior; real devices and consuming apps prove camera, networking, and kernel compatibility. Mocks cannot establish hardware support, performance, background survival, or shell stability.

Each result records date, commit, dependency versions, host/kernel/GPU, phone/OEM/OS, network topology, input/output modes, procedure, expected result, actual result, and a sanitized artifact. Missing hardware means **not tested**, never passed. Define supported versions from this evidence before release.

## Requirement acceptance matrix

| Test | Requirements | Scenario and observable pass condition |
| --- | --- | --- |
| T01 Setup | R01, R19 | Clean and partially configured hosts show precise readiness; installation cancellation leaves existing services/configuration intact |
| T02 UI lifecycle | R02, R09 | Hide/reopen widget and panel during stream; one phone capture and one desktop decode remain active |
| T03 Pairing attacks | R03, R04, R17 | Expired/replayed/raced invitation, spoofed discovery, wrong key, cancellation, and key replacement cannot establish unauthorized trust |
| T04 Consent | R05, R13, R20 | Discovery, pairing, boot, daemon restart, and Auto-connect never start fresh capture without phone consent |
| T05 Exclusive phone | R06 | Concurrent starts or two trusted phones produce one active owner; second request gets a visible conflict |
| T06 Capabilities | R07, R08 | Test different capability sets, stale revision, invalid FPS/size tuple, camera busy, and failing lens switch; no fabricated modes or unhandled failure |
| T07 Shared pipeline | R09 | Open multiple preview surfaces and consumers; count capture/encoder/decoder instances and verify bounded frame memory |
| T08 Consumer output | R10, R11 | Chrome/Chromium, Firefox, OBS, and one Electron/WebRTC app enumerate and consume OmaCam Camera; record each exact version and packaging form |
| T09 Reconnect | R11, R13 | Interrupt local path for up to 2 seconds, restore it, and measure fresh output recovery; handle/device persists while writer stays healthy |
| T10 Long loss | R05, R13 | Disconnect beyond 10-second lease; phone releases camera; restoring the network requires a new Start |
| T11 Privacy stop | R05, R17, R20 | Stop/Forget/lock while recognizable image is present; neutral output within 500 ms locally; stale frames never reappear; unreachable phone stops by lease expiry |
| T12 Quality | R08, R10, R12 | Degrade bandwidth using isolated test infrastructure; applied source quality changes within capability bounds; output format stays fixed; queues remain bounded |
| T13 Black screen | R14 | Run dim UI with accessible reveal/Stop; camera stays active only as allowed; OS indicators remain; resources release on stop |
| T14 Audio exclusion | R15 | Inspect permissions and device/resource use; no microphone capture in MVP, including any future adapter's defaults |
| T15 USB coexistence | R16 | Begin with working USB tethering and ongoing traffic; pair/start/stop/reconnect OmaCam through a supported path; no USB/profile/route mutation or attributable interface reset |
| T16 Hotspot coexistence | R16 | Phone hotspot supplies laptop internet; perform camera flow if reachable, or report unsupported path; hotspot and internet remain enabled |
| T17 Other networking | R16 | Existing VPN, Wi-Fi, Ethernet, firewall, and client-isolated network; fail safely if unreachable; no automatic reconfiguration |
| T18 Diagnostics | R18 | Inject each stage failure; report correct stage, sanitize secrets/addresses/names, export no media, run no hidden repairs |
| T19 Packaging lifecycle | R19 | Install, interrupted install, update, incompatible kernel, rollback, uninstall, and modified-file conflict; preserve unrelated devices and shared packages |
| T20 Session lifecycle | R05, R13, R20 | Lock, suspend, logout, companion death, controller death; no silent restart and no stale output; user-visible recovery |
| T21 Accessibility | R02, R07, R14 | Keyboard-only desktop flow, readable status without color, enlarged text, reduced motion, accessible phone Stop |

Desktop lock and suspend detection must be tested on the target shell/session implementation. Failure to observe a required privacy event is a blocker, not grounds to remove the rule.

## Hardware and consumer qualification

Start with at least two real Android devices from different OEMs: one baseline and one with additional exposed camera capabilities. Record front/rear behavior, available controls, OS permission behavior, heat, and background/black-screen limits. A broader compatibility claim needs more devices; do not generalize from one flagship phone.

Qualify at least one host/kernel/GPU combination for the first release. Add an alternate supported kernel and a host with an existing loopback user to the packaging matrix before claiming those scenarios supported. Include software-decode fallback measurements when hardware decode is absent.

For apps, record native package and sandboxed/Flatpak variants separately. Portal-based camera selection is a separate test from direct V4L2 access; see [S7](09-decisions-and-sources.md#s7-camera-portal). Confirm multiple consumers can open the same output at the supported mode. Never assume Meet, Zoom, Discord, or Electron products behave identically because they share technology.

## Performance measurement

- **Startup:** measure from accepted Start with all permissions already available to first fresh preview and first fresh consumer frame. Run at least 30 trials; report median, p95, failures, and cold/warm conditions separately. First installation and user approval time are separate usability measures.
- **Latency:** film a physical changing timer/light and the consumer display in the same high-frame-rate recording, or use an equivalent calibrated optical method. Record display/capture measurement resolution and uncertainty. Sample at least 100 transitions; report median and p95. Do not use transport RTT as this result.
- **Recovery:** run at least 30 controlled outages; start timing when reachability returns and stop when a fresh consumer frame appears. Record outages that require consent separately.
- **Resources:** measure steady idle and streaming CPU, RSS, GPU use, queue depth, dropped/late frames, output FPS, temperature/throttling, and battery conditions. CPU percentages use one-core units. Compare after warmup and at the end of the 60-minute run; investigate continued memory growth rather than judging only a single snapshot.
- **Coexistence:** record networking/USB state before, during, and after operation and keep a concurrent internet transfer or connectivity probe running. Compare packet loss/throughput with an idle baseline; performance contention may occur, but configuration changes and attributable disconnects fail the test.

Network impairment tests must use a dedicated test interface, namespace, router, or lab system with explicit authorization. Never change the user's live default route or firewall to manufacture a benchmark.

## Failure injection checklist

Test decoder/media-worker crash, writer crash, control-daemon crash, slow preview, stalled consumer, malformed/oversized packet, late frame, full storage during trust update, interrupted migration, missing secret store, certificate/key mismatch, camera permission revocation, thermal shutdown, interface address change, USB removal, and a second application already using the phone camera.

Pass means safe state, bounded resources, accurate error, and documented recovery. Not every crash promises uninterrupted webcam visibility. In particular, writer and kernel failures have the weaker persistence guarantee defined in the PRD.

## Packaging and system ownership

1. Ship a versioned Arch package for binaries and declared system integration, and an Omarchy-compatible plugin artifact with its manifest at the installed plugin root. The plan's nested source layout needs an explicit packaging/export step.
2. Record tested kernel/header/module combinations. Detect module build/signature failures and provide instructions; never disable Secure Boot or unload another user's module.
3. Allocate a collision-free owned loopback device. Track ownership independently of device number and card label. If the module is already loaded, do not replace its global options; qualify scoped dynamic device creation or report unsupported setup.
4. Provision least-privilege local access using the tested host model. Do not grant all users camera access or blanket-write video nodes. Multi-user hosts need an explicit device/access ownership policy before being supported; baseline may be one active desktop user.
5. Use user services with intentional lifecycle. No lingering capture after logout; no root media service. Do not assume a package installation can safely enable a service in every user's session.
6. Preserve an ownership receipt for changed paths and prior content where applicable. Privileged writes validate target and symlink safety. Interrupted setup is rerunnable and does not duplicate devices or config entries.
7. Test plugin/daemon/phone version skew. A plugin update cannot assume a matching system-package update; mismatches need a useful readiness message.

## Repair, update, rollback, and removal

Read-only diagnostics precede any repair. Show the exact failing stage, proposed owned changes, required privileges, and recovery. Prefer restarting only an OmaCam-owned failed component. Changes to shared services are outside automatic repair.

For upgrades, stage and verify artifacts before stopping an active session; ask the user to end a call before an update that disrupts output. Keep signing identities stable. Back up owned configuration before migration and document whether old versions can read it. Do not roll back a binary into an incompatible or insecure trust store.

Uninstall stops only OmaCam-owned processes, removes only proven owned integration and unused owned devices, and leaves shared dependencies in place. If a device is still in use, defer deletion with an explanation. Offer a separate option to remove trust/keys; never delete a whole desktop configuration directory. Preserve modified files and explain any leftovers. Phone trust removal is a separate action when the phone is unavailable.

## Release checklist

- Every MVP requirement R01–R20 has recorded test evidence; no safety or coexistence failure is waived as a performance tradeoff.
- G0–G7 deliverables are complete; proposed scope and platform decisions have been resolved.
- Exact supported versions and unsupported configurations are published.
- No hidden recording, microphone access, plaintext fallback, external relay, or automatic system reconfiguration exists.
- Dependency integrity, licenses/notices, Android signing, and repeatable clean builds are documented for the selected artifacts.
- Upgrade, rollback, interrupted install, and ownership-preserving uninstall are rehearsed.
- Documentation matches actual shipped functionality; performance claims cite measured results and conditions.
