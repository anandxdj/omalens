# OmaCam product requirements

Version: proposed baseline 0.1. Audience: product owner, implementing LLMs, maintainers, and testers.

## Problem and intended users

An Omarchy user wants better video calls using the phone they already own, without configuring camera devices, networking, codecs, or developer tools. The primary user joins meetings; the secondary user selects the camera in OBS or another capture application. Neither needs recording or streaming production features in OmaCam.

The product succeeds when a user installs supported components, pairs a phone, explicitly starts its camera, sees a preview, and selects **OmaCam Camera** in another app while their existing internet connection keeps working.

## Release scope

| Release | Included | Not required for that release |
| --- | --- | --- |
| MVP / first usable release | Android companion; QR pairing; one active phone; local IP transport over an existing reachable network; front/rear selection where exposed; supported zoom, exposure compensation, and torch; 720p30 baseline on qualified hardware; shared-source preview; persistent V4L2 output while the output service is healthy; reconnect; black-screen UI; native Omarchy panel; scoped diagnostics and installation | Browser client, scrcpy, native UVC, custom USB protocol, phone audio, automatic provider handover, manual sensor controls, universal 1080p60 |
| Expansion A | Existing USB-tether IP path qualification; UVC or scrcpy provider selected by evidence; more devices and quality profiles; accessible physical-lens controls; safe provider switching | Simultaneous capture from multiple phones |
| Expansion B | Browser/PWA where secure onboarding is solved; opt-in phone microphone and synchronization; advanced controls; automatic safe handover; additional desktop frontends | Cloud media relay, recording, remote surveillance |

USB-tether and hotspot **non-interference tests are MVP requirements**, even when streaming over that specific route has not been qualified. A refused unsupported route is acceptable; breaking working networking is not. Do not market a transport as supported until it has passed its functional tests.

Recommended qualification starting point: Omarchy with the documented Quickshell plugin API, x86-64 Linux, and Android 12 or later. These are proposed support boundaries, not API-derived universal guarantees. Gate G0 records exact versions, target/minimum SDK choices, kernels, phone models, and browsers. Broader platform support is evidence-driven.

## Functional requirements

| ID | Requirement and acceptance outcome |
| --- | --- |
| R01 | Onboarding detects missing components and separates installation from pairing. A missing module must never appear as “phone unavailable.” |
| R02 | A user can open the panel with or without a bar widget. Hiding the widget does not stop an explicitly active stream. |
| R03 | QR pairing uses a fresh short-lived session and authenticates the intended peer. No media starts from scanning alone. Expired and replayed invitations fail. |
| R04 | Both peers retain device identity and user-visible names. Ask is the default connection policy. Rename does not change identity; rediscovery does not establish trust. |
| R05 | Starting a session requires explicit capture consent on the phone. The laptop can request capture; the phone handles foreground and permission requirements. Stop from either side ends capture. |
| R06 | Exactly one phone owns the capture session. A second request cannot silently replace the active phone. A user-initiated switch stops the old session first. |
| R07 | Camera choices and controls reflect current usable capabilities. Friendly fallback labels are acceptable; nonexistent telephoto or ultrawide options are not. |
| R08 | Unsupported combinations are rejected with a supported alternative. Failed lens changes either restore the previous camera or enter a visible safe failure state. |
| R09 | The companion produces one encoded camera stream. Desktop preview and virtual output share the same received source; opening a panel must not start another phone stream. |
| R10 | One OmaCam-owned virtual device is labeled OmaCam Camera. Its output format remains stable during a ready service session; upstream quality changes are normalized. |
| R11 | While the output writer remains healthy, network loss, phone disconnect, and lens switching retain the device and feed neutral frames during gaps. Consumer apps stay attached in qualified tests. |
| R12 | Automatic quality reduces bitrate or input quality under sustained load. It stays within negotiated modes and preserves the output contract. Manual changes show actual applied values. |
| R13 | Reconnection authenticates the same peer and cannot undo a Stop, Forget, lock, or expired capture lease. Session policy is defined in the state contract. |
| R14 | Black-screen mode dims only the companion's own active UI and provides an accessible way to reveal Stop. It does not claim to be Android lock or disable OS privacy indicators. |
| R15 | Microphone access is absent from the MVP capture path. Later phone audio is explicitly opt-in and never changes the system or meeting application's default input. |
| R16 | Existing tethering, hotspot, USB functions, DNS, routes, firewall, VPN, and other cameras remain under their existing owners. Unsupported conditions produce guidance rather than forced repair. |
| R17 | Forget revokes future authentication and any current session locally, deletes applicable credentials, and explains that the disconnected peer may need to be forgotten separately. |
| R18 | Diagnostics identify discovery, authentication, capture, transport, decode, and output failures separately. Exports are redacted and initiated by the user. |
| R19 | Install, upgrade, repair, and uninstall respect shared dependencies and user-modified files. Runtime operates without root. |
| R20 | Camera status is visible on the phone and in the desktop UI. Desktop lock, logout, and suspend stop capture and disarm automatic capture recovery. Reopening requires a new Start. |

## UX flow and error behavior

1. Open OmaCam. Run a read-only readiness check. If setup is incomplete, explain the missing component and show the supported installation path.
2. If no trusted phone is selected, show pairing instructions. For MVP: install/open the companion and scan the desktop QR with its scanner. Do not pretend the generic Android camera opens a zero-install client.
3. Show the prospective peer on both screens and complete the pairing approval ceremony. Request camera permission only as needed for scanning or capture, with the reason shown.
4. Request Start. The phone shows the intended laptop and asks to share its camera. On approval, expose actual capture settings and preview; output becomes live only after readiness.
5. Show camera selector, Automatic quality, current state, preview, and Disconnect. Advanced diagnostics are collapsed. Do not include inactive future-feature controls.
6. Tell the user to select OmaCam Camera in the meeting app. OmaCam cannot force the meeting app to switch inputs.

| Situation | User-visible behavior | Recovery |
| --- | --- | --- |
| Unknown / unavailable phone | “Phone not found” | Open companion; verify both devices can reach the same local network |
| Approval needed | “Continue on your phone” | No repeated permission prompts from the laptop |
| Camera busy | “Camera is being used by another app” | User releases it; OmaCam does not terminate the other app |
| Transport lost | “Reconnecting…” and neutral output | Bounded authenticated retry within an armed session |
| Output unavailable | “Phone connected; virtual camera unavailable” | Scoped diagnostics; never claim ready from preview alone |
| No compatible mode | “This camera cannot use the selected quality” | Offer validated alternatives |
| Desktop locked / session stopped | “Camera stopped” after unlock | Explicit new Start |
| Identity changed | “Device identity changed; pair again” | No silent replacement of the stored identity |

All actions must be keyboard-accessible. Status must use text as well as color, respect reduced motion, and remain understandable at desktop scaling and enlarged text. Preview mirroring is local preference; outgoing video defaults to unmirrored, correctly rotated video. Preserve aspect ratio with letterboxing rather than silent cropping.

## Performance targets and failure envelope

These proposed targets apply to qualified hardware and supported modes; G0 must record a reference rig and a repeatable protocol before performance work is accepted.

| Measure | Initial target | Conditions |
| --- | --- | --- |
| Known-session startup | p95 ≤ 2 seconds | Phone already open, permissions granted, peer reachable; measure Start to first fresh frame at output and preview separately |
| Wireless camera-to-consumer latency | Median < 100 ms; p95 < 150 ms | Strong local network, baseline mode; optical measurement rather than RTT |
| Future wired latency | Median < 60 ms; stretch < 40 ms | Qualified wired provider, same optical method |
| Brief interruption recovery | p95 ≤ 3 seconds after reachability returns | Up to 2 seconds of outage, same armed session; no user stop or OS permission loss |
| Stale-image removal | ≤ 500 ms after Stop, Forget, or detected link loss; also after 500 ms without a fresh acceptable source frame | No captured frame remains at output; remote stop also governed by lease timeout |
| Output service idle resources | Mean < 1% of one CPU core; RSS ≤ 100 MiB | Ten-minute steady state, no phone capture; report writer included/excluded |
| Sustained use | 60-minute baseline call without fatal error or unbounded queue growth | Track thermal throttling, drops, memory, and actual FPS |

If targets fail, record the result and make a product decision; do not relabel measurements or silently weaken thresholds. Streaming CPU/GPU budgets must be set against the selected rig in G0.

Persistence covers a healthy writer across capture and transport failures. Writer crash, full service restart, kernel/module failure, reboot, and consumer bugs may require reselection; report these honestly. Automatic recovery must still return to a safe state without unauthorized capture.

## Explicit exclusions

No recording, snapshots, screen mirroring, file transfer, remote desktop, cloud media, surveillance, simultaneous phone streams, root on Android, or disabling operating-system security controls. No claim of supporting all Omarchy releases, Android devices, lenses, browsers, or sandboxed apps without qualification.
