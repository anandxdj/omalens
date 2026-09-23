# OmaCam completion and qualification plan

Status: approved execution plan; no open gate is complete until its evidence is
recorded. Created 2026-09-10 from the PRD, roadmap, verification matrix, G5-004
handoff, and packaging report.

## Objective

Finish the five open areas in this order:

1. real phone-to-virtual-camera operation;
2. live Omarchy plugin verification;
3. MVP onboarding and product controls;
4. reliability and long-duration qualification;
5. reproducible, identified, signed release artifacts.

The release candidate is complete only when G4-G7 exit evidence exists and all
applicable MVP requirements R01-R20 have a recorded result. Automated or
synthetic tests are prerequisites, not substitutes for hardware, shell,
consumer-application, or lifecycle evidence.

## Locked execution preferences

- **Build mode:** step-by-step, following the numbered checklist below.
- **Verification:** mandatory at every checkpoint; do not batch unverified work
  across a gate.
- **Check-ins:** stop after each numbered item and summarize pass/fail evidence,
  changed files, and the next item.
- **Git:** preserve the current dirty tree. Commit only after the user reviews a
  completed checkpoint; never mix unrelated pre-existing changes into a commit.
- **Hardware/system safety:** identify exact targets before every privileged or
  live action. Never use the integrated `/dev/video0` or `/dev/video1`, change
  networking/firewall/USB state automatically, or restart the live shell without
  explicit approval for that action.
- **Failure policy:** a failed target is recorded as failed. Fix the cause and
  rerun the narrowest affected test, then its parent gate; do not weaken an
  acceptance threshold silently.

## Release boundary for controls

The MVP must expose camera choice, Automatic quality, preview mirroring,
black-screen mode, and the actual applied state. Zoom, exposure compensation,
and torch may ship in the same release only when advertised by the active phone,
round-trip acknowledged, failure-safe, and tested on real capability fixtures.
Otherwise they remain hidden and become the first post-MVP control slice. This
resolves the difference between the broad G5 roadmap wording and the narrower
MVP scope in the PRD without showing controls that do not work.

## Evidence layout

Create one dated directory per qualification run:

```text
docs/implementation/evidence/YYYY-MM-DD-<short-run-name>/
  environment.md
  procedure.md
  results.md
  artifacts/
```

Every `results.md` records: commit and dirty-tree identity, host/kernel/GPU,
Omarchy/Quickshell/Qt versions, package versions, phone/OEM/Android version,
network topology, output node ownership, consumer versions, expected result,
actual result, timings, sanitized logs, failures, cleanup, and final verdict.
Never store keys, pairing secrets, full addresses, unredacted device names, or
camera frames that the user did not explicitly choose to retain.

## Preconditions

- [ ] A test phone running Android 12 or later is available by USB for
  instrumentation and by a supported local network for the product flow.
- [ ] A second phone from another OEM is scheduled before release capability
  claims are finalized.
- [ ] A dedicated OmaCam-owned V4L2 loopback device can be created without
  modifying or replacing existing loopback ownership/options.
- [ ] At least OBS and one browser consumer are installed; exact versions and
  packaging forms are recorded.
- [ ] The current worktree is snapshotted with `git status --short`,
  `git diff --check`, and a patch or commit identifier before live tests.
- [ ] The user explicitly approves each privileged install/device action and
  each live Omarchy plugin load/reload.

If any precondition is unavailable, mark the dependent item blocked and continue
only independent work. Missing hardware is never a pass.

## Sequenced checklist

- [ ] **1. Freeze the qualification baseline**
  Spec ref: `docs/07-verification-and-release.md > Evidence rules`; G0/G7.
  What to do: Record the exact host, shell, kernel, graphics, GStreamer, Rust,
  JDK, Android SDK, phone, network, consumers, current repository state, and
  existing video/loopback ownership. Choose the one supported reference rig.
  Acceptance: The run is reproducible and no existing camera, loopback user,
  route, firewall rule, USB function, or service has been changed.
  Verify: Run read-only inventory plus `cargo run -p omacam-daemon -- doctor`;
  review `environment.md` and confirm every intended mutable target is named.

- [ ] **2. Re-establish the automated green baseline**
  Spec ref: G4/G5 implementation handoffs and T03-T07, T11, T14, T18.
  What to do: Run formatting, Rust checks/tests/Clippy/release build, QML lint,
  Android JVM tests, debug/release builds, lint, instrumentation APK build,
  package build, archive inspection, and isolated lifecycle test.
  Acceptance: All existing automated checks pass from the exact candidate tree;
  test counts and tool versions are recorded. Up-to-date Gradle tasks are rerun
  when evidence requires fresh execution.
  Verify:

  ```bash
  cargo fmt --all -- --check
  cargo check --workspace --all-targets
  cargo test --workspace --all-targets
  cargo clippy --workspace --all-targets -- -D warnings
  cargo build --release --locked --workspace
  qmllint -I /usr/share/omarchy/shell omarchy-plugin/*.qml
  JAVA_HOME=/usr/lib/jvm/java-17-openjdk ANDROID_HOME=/tmp/omacam-android-sdk \
    ./android/gradlew -p android testDebugUnitTest assembleDebug \
    assembleDebugAndroidTest assembleRelease lintDebug lintRelease --rerun-tasks
  makepkg --force --noconfirm --cleanbuild --dir packaging/arch
  bash packaging/tests/install-lifecycle.sh
  ```

- [ ] **3. Run connected Android lifecycle tests**
  Spec ref: T03-T06, T10-T11, T20; R03-R08, R13, R17, R20.
  What to do: Attach the baseline phone, confirm its identity, install only the
  debug test artifacts, and run the instrumented lifecycle suite. Then manually
  exercise approval, decline, permission denial/revocation, camera busy,
  duplicate callbacks, phone Stop, desktop Stop, Forget, app death, and restart.
  Acceptance: Capture never starts without explicit consent; invalidation wins
  over late callbacks; cleanup releases camera/codec resources; errors are
  accurate and recoverable; no microphone permission or capture exists.
  Verify:

  ```bash
  adb devices -l
  JAVA_HOME=/usr/lib/jvm/java-17-openjdk ANDROID_HOME=/tmp/omacam-android-sdk \
    ./android/gradlew -p android connectedDebugAndroidTest
  ```

  Record the instrumentation XML plus a manual result for each scenario.

- [ ] **4. Provision and prove one owned virtual camera**
  Spec ref: R10-R11, R16, R19; T01, T08, T15-T17, T19.
  What to do: Design and execute the least-privilege provisioning path for one
  collision-free device labeled `OmaCam Camera`. Record ownership independently
  of device number. Validate install, repeat install, partial failure, repair,
  rollback, and removal without touching unrelated devices or shared module
  options.
  Acceptance: The node is provably OmaCam-owned, fixed at 1280x720 YUY2 30 fps,
  accessible to the intended user, neutral while idle, and safely removable;
  existing `/dev/video0` and `/dev/video1` remain untouched.
  Verify: Compare `v4l2-ctl --list-devices`, node metadata, module parameters,
  permissions, service state, and ownership receipt before/during/after. Run
  `v4l2-ctl --device <owned-node> --all` only after ownership is proven.

- [ ] **5. Complete the real phone-to-consumer happy path**
  Spec ref: G4 exit; R03-R05, R09-R11, R15, R20; T04, T07-T08, T11, T14.
  What to do: Pair, authenticate, request Start, approve on phone, receive one
  720p30 stream, render preview, and feed the owned V4L2 node. Test OBS, Chromium,
  Firefox, and one Electron/WebRTC consumer when available.
  Acceptance: Recognizable correctly rotated imagery reaches preview and every
  qualified consumer; there is one phone capture/encoder and one desktop
  decoder; audio is absent; Stop produces neutral output within 500 ms and
  releases phone resources within 10 seconds while the consumer handle remains.
  Verify: Record consumer enumeration/capture, process topology, negotiated caps,
  Stop timing, resource release, and sanitized service/phone logs. Complete one
  uninterrupted 30-minute G4 trial before product controls are enabled.

- [ ] **6. Qualify the existing Omarchy panel in a controlled live session**
  Spec ref: R02, R09, R18, R20; T02, T18, T21.
  What to do: Install the repository plugin artifact at the supported plugin
  root, enable one bar-widget instance, reload through the supported Omarchy
  mechanism, and exercise service unavailable, setup needed, unpaired, offline,
  ready, awaiting approval, streaming, stopping, failed, and diagnostics states.
  Acceptance: Manifest discovery and QML loading produce no warnings/errors;
  the bar icon and panel render at supported scaling; Start/Stop/Forget work;
  preview opens/closes/reopens without another capture or decoder; lost/restarted
  revision streams recover; keyboard-only and accessibility actions work; plugin
  removal restores prior configuration.
  Verify: Capture sanitized Quickshell logs, screenshots for every principal
  state, process counts before/after preview reopen, keyboard walkthrough, and
  exact install/remove diff. This item requires explicit approval before the
  live plugin/configuration change or shell reload.

- [ ] **7. Finish onboarding and capability-driven controls**
  Spec ref: G5; R01-R02, R06-R08, R12, R14, R18-R20.
  What to build: Add terminal-free readiness/provisioning guidance, provider and
  pairing entry, camera selector, Automatic quality, preview mirroring,
  black-screen mode, and applied-value/error feedback. Add zoom, exposure, and
  torch only under the release-boundary rule above. Keep output unmirrored and
  preserve aspect ratio.
  Acceptance: A new user can go from missing prerequisites to pairing and a call
  without technical identifiers; unsupported controls are absent; stale
  capabilities cannot be applied; failed switching restores the previous camera
  or enters a visible safe state; black-screen mode retains an accessible Stop
  and Android privacy indicators.
  Verify: Unit-test capability/state transformations, integration-test command
  acknowledgement and stale revisions, rerun QML/Android checks, then perform
  manual flows on the baseline and second phone with screenshots and actual
  applied-value records.

- [ ] **8. Complete failure, reconnect, and lifecycle qualification**
  Spec ref: G6; T03-T07, T09-T12, T18, T20.
  What to do: Exercise brief and long network loss, stale/replayed media, queue
  pressure, slow preview, stalled consumer, decoder/output/daemon/companion
  death, lock, suspend, logout, interface churn, permission revocation, camera
  contention, and thermal shutdown. Use only isolated impairment infrastructure.
  Acceptance: Resources remain bounded; no stale image returns; brief recovery
  meets p95 <= 3 seconds across 30 trials; long loss expires the lease and needs
  a new Start; privacy events stop capture; failures identify the correct stage;
  no automatic network, firewall, route, USB, or unrelated-camera mutation occurs.
  Verify: Automated fault tests where deterministic, plus dated traces and 30-run
  recovery statistics. Rerun the happy path after every repaired failure class.

- [ ] **9. Measure performance and complete sustained trials**
  Spec ref: `docs/02-prd.md > Performance targets`; G6 exit; T12.
  What to do: Run 30 cold/warm startup trials, at least 100 optical latency
  transitions, 30 recovery trials, a 10-minute idle resource sample, a 30-minute
  call, and the final 60-minute session. Track CPU, RSS, GPU, queue depth,
  drops/late frames, FPS, temperature, throttling, and phone battery conditions.
  Acceptance: Startup p95 <= 2 s, wireless latency median < 100 ms and p95 <
  150 ms, stale imagery <= 500 ms, idle mean < 1% of one core and RSS <= 100 MiB,
  and 60 minutes without fatal error or unbounded growth—or an explicit product
  decision documents any missed target before release.
  Verify: Preserve raw sanitized samples, calculation method, uncertainty,
  median/p95 output, graphs or tables, and before/after resource snapshots.

- [ ] **10. Complete coexistence and compatibility coverage**
  Spec ref: R07-R08, R15-R16; T06, T08, T14-T17, T21.
  What to do: Repeat critical flows with a second-OEM phone, USB tethering active,
  phone hotspot active, VPN/firewall present, and another existing camera or
  loopback owner. Test native and sandboxed consumer variants separately where
  release claims include them.
  Acceptance: Supported combinations work; unsupported routes fail with precise
  guidance; internet connectivity and existing owners remain intact; capability
  labels and controls match each phone; no microphone access appears.
  Verify: Record before/during/after network and USB state, concurrent connectivity
  probe, supported/unsupported matrix, consumer versions, and permission/resource
  inspection.

- [ ] **11. Finalize release identity, packaging, signing, and lifecycle**
  Spec ref: G7; R01, R18-R19; T01, T18-T19.
  What to build: Resolve public name, application/package IDs, repository URL,
  maintainer, license/notices, stable desktop and Android signing identities,
  provenance, dependency integrity, update/version-skew behavior, compatibility
  matrix, ownership receipts, rollback, and uninstall documentation. Produce
  reproducible Arch and Android release artifacts without installing or enabling
  services as a package side effect.
  Acceptance: Clean builds are repeatable; artifacts are signed and traceable;
  plugin/daemon/phone skew fails usefully; install/upgrade/interruption/rollback/
  uninstall preserve user-modified and unrelated data; support boundaries and
  known limitations are published.
  Verify: Compare hashes from two clean builds where feasible, verify signatures,
  inspect archive/APK contents and permissions, run rootful lifecycle checks in
  an isolated disposable environment, and review every owned path before/after.

- [ ] **12. Run the release gate and freeze evidence**
  Spec ref: `docs/07-verification-and-release.md > Release checklist`; G7 exit.
  What to do: Trace R01-R20 and T01-T21 to dated results, close or explicitly
  defer every failure, rerun the complete automated matrix from the release tag,
  execute one final happy path using the signed artifacts, and update README,
  support matrix, limitations, install/upgrade/uninstall, and recovery docs.
  Acceptance: No required cell lacks evidence; no safety/coexistence failure is
  waived; documentation matches shipped behavior; open expansions are not
  advertised; the release tag and artifact hashes identify the tested bits.
  Verify: A reviewer can follow the traceability table from each requirement to
  command/manual evidence and reproduce installation, one call, Stop, and clean
  removal from the signed artifacts.

## Gate order and pause points

| Gate | Items | Proceed only when |
| --- | --- | --- |
| A — Baseline | 1-3 | Automated and connected-phone lifecycle checks pass |
| B — Real media | 4-5 | Owned output and 30-minute phone-to-consumer flow pass |
| C — Product UI | 6-7 | Live shell, onboarding, controls, and accessibility pass |
| D — Reliability | 8-10 | Failure, performance, sustained, and coexistence evidence pass |
| E — Release | 11-12 | Signed lifecycle-tested artifacts and full traceability pass |

Stop for user review at every gate. Do not start Gate C from synthetic media
alone, Gate D from an unqualified UI, or signing/release publication before the
supported matrix and ownership model are settled.

## Definition of done for the five requested areas

| Area | Done when |
| --- | --- |
| Real phone-to-virtual-camera | Items 3-5 pass on an owned output and qualified consumers |
| Live Omarchy UI | Item 6 passes with reversible installation and live-shell evidence |
| Product controls/onboarding | Item 7 passes on two capability fixtures with unsupported controls hidden |
| Reliability/long-duration | Items 8-10 pass, including measured 30/60-minute trials |
| Release packaging/signing | Items 11-12 pass with signed, reproducible, lifecycle-tested artifacts |

## First execution step

Begin with Item 1 only. Record the baseline and exact mutable targets, report
the result, and wait for review before running connected-device, privileged,
V4L2, live-shell, or installation actions.
