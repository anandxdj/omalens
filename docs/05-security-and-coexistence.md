# Security, privacy, and coexistence rules

These requirements are release gates. They guide implementation; they are not a claim that an unimplemented system is secure.

## Threat model

| Threat | Protected asset | Required defense and verification |
| --- | --- | --- |
| Malicious device on LAN or spoofed mDNS record | Peer trust, camera access | Authenticate identities; discovery grants no trust; spoofing test |
| Photographed QR or replayed invitation | Pairing authorization | Short lifetime, one-time claim, visible peer approval, transcript binding; replay/race tests |
| MITM during signaling or reconnect | Media confidentiality and control integrity | Maintained authenticated cryptography, peer-key verification, session/media binding; interception test |
| Forgotten or replaced phone | Future access | Local revocation invalidates active and cached sessions; reconnect/key-change tests |
| Malformed control or media | Desktop availability | Parse limits, bounded allocation, isolated native decoding, timeouts; fuzz and malformed-input tests |
| Unexpected teardown, lock, or network loss | Camera privacy | Clear stale output, release capture/torch, short phone lease; fault injection |
| Unsafe installer or repair | Working desktop/network | Minimal fixed-purpose privileges, ownership checks, reversible changes; configuration diff tests |
| Log or diagnostic disclosure | Secrets, images, device privacy | Redaction by construction, no media dumps, bounded retention; automated export inspection |

Out of scope: defending against a rooted/compromised phone, a hostile kernel, or malware already controlling the user's desktop account. Do not use this limitation to excuse unauthenticated LAN access or broad root privileges.

## Pairing ceremony

1. Desktop creates an invitation with the bounds in the state contract. Its QR is an authorization-bearing secret: render locally and exclude it from logs, analytics, crash reports, and persistent files.
2. Companion scans inside its own app for MVP. Treat every field as untrusted input. Accept only the defined scheme/encoding and bounded endpoint forms; never execute or open arbitrary QR-supplied commands or URLs.
3. Establish a channel using a maintained authenticated protocol implementation. Bind the laptop identity to the QR and prove possession of both peers' identity keys. The ephemeral invitation proof and entire handshake transcript must be bound to that channel.
4. Present both device names and a library-derived short authentication string on both screens. The user confirms that the values match. This provides a check against an unintended claimant; names alone do not.
5. Consume the invitation atomically, require approval on both peers, then commit trust records. No media before a separate Start authorization. Cancellation, expiry, or partial failure invalidates the pending ceremony; partial local trust must never authorize capture by itself.
6. Future sessions verify the stored identity, fresh session proof, and protocol policy. Rotate ephemeral session material; never reuse invitation secrets as permanent credentials.

G1 must select the exact cryptographic library/protocol, key types, transcript definition, authenticated media binding, and key-storage behavior and subject them to review before implementation. These prose steps are not instructions to design a new cryptographic handshake. A pinned native certificate/key bootstrap must be implemented as explicit peer verification, never a generic “accept all certificates” callback.

For WebRTC, encrypted media alone is insufficient: signaling and the media certificate fingerprint must be bound to the authenticated peer/session. Do not introduce public STUN/TURN, a cloud signaling requirement, or arbitrary remote endpoints to make local tests pass.

## Capture consent and privacy

- Pairing, transport connection, OS camera permission, and current capture consent are distinct facts.
- Default to Ask. Even Auto-connect cannot silently arm a fresh capture session.
- Stop from the phone is always available, including black-screen mode and an accessible foreground notification where required.
- Desktop lock, logout, suspend, Stop, and Forget invalidate the desktop lease. If the phone cannot receive Stop, it must stop on lease expiry within 10 seconds. Local output clears within the PRD budget.
- No microphone permission, recording, screenshots, frame dumps, or telemetry in MVP. QR scanning may require camera permission before streaming; explain that use honestly.
- Clear previews and all output queues on disarm. Never freeze a user's last captured image as a disconnection placeholder.
- Release torch, camera, encoder, foreground service, and wake resources after stop or error. Restore only settings changed by the app within its own ownership scope.
- Black-screen mode cannot bypass Android background rules or hide OS camera indicators. Use supported lifecycle APIs and test restrictions on each qualified OS/OEM. See [S5](09-decisions-and-sources.md#s5-android-lifecycle).

## Forbidden coexistence actions

Runtime and automatic repair MUST NOT:

- Change USB gadget functions, enable ADB or wireless debugging, reset USB devices, or toggle tethering/hotspot.
- Disable or recreate Wi-Fi connections, change routing metrics/default routes/DNS, flush firewall rules, alter VPN policy, or disable client isolation.
- Kill a shared ADB server, broad process names, another camera app, PipeWire, NetworkManager, or the desktop shell to obtain resources.
- Unload a shared v4l2loopback module, overwrite its global options, take another application's virtual camera, or chmod all video devices.
- Disable Secure Boot, signature checks, browser certificate verification, SELinux/AppArmor policies, or Android battery/security protections.
- Change default microphones, remove shared dependencies, or replace unrelated shell configuration.

Unknown disruption risk means reject the candidate. Prefer an existing working connection. Changing the active transport solely for a small latency improvement is deferred until handover has its own evidence and rollback design.

These prohibitions apply to automatic operation. A future user-requested system change requires a separate, explicit workflow describing the exact effect; ordinary pairing consent does not authorize it.

## Privilege boundaries and repair

The daemon, media worker, writer, and UI run as the desktop user. Kernel/device provisioning belongs to a narrowly scoped package/setup workflow. If a helper is unavoidable, define a fixed operation list with validated arguments and authorization; never accept arbitrary shell text, file paths, module parameters, or device paths from the UI/network.

Repair starts with read-only diagnostics. It returns findings and an exact proposed change list with ownership evidence and rollback. A user can authorize an OmaCam-owned service restart or privileged setup step; “Repair” must not become unlimited system control. Reuse authorization already supplied for the specific operation, but do not infer unrelated repairs from it.

Before removing a device or file, verify ownership and current content. A matching label is insufficient. Preserve user modifications and shared state; if ownership cannot be proved, leave it and explain the manual next step.

## Network and local resource restrictions

Limit listeners to intended local interfaces and the pairing/session lifecycle. Define IPv4/IPv6 scope handling explicitly; block arbitrary public, multicast, broadcast, and metadata endpoints in invitations unless a documented protocol step requires them. Treat user-selected VPN/tunnel paths as a separate explicit support decision, not proof that every reachable address is local.

Use maintained parsers, deadlines, input size limits, per-peer quotas, and authenticated control before media negotiation. Do not scan subnets or download executable dependencies from a peer. Spawn fixed binaries with argument arrays and explicit process ownership; never interpolate device names into a shell command.

Use restricted per-user runtime paths and safe file replacement; avoid symlink-following privileged writes. A future diagnostics export must sanitize names, addresses, keys, tokens, and identifiers and remain local unless the user chooses to share it.

## Update trust

Select an Android signing-key custody and distribution process before distributing an APK. Never generate a replacement release key to bypass an upgrade problem. Package and plugin provenance, dependency locks, integrity checks, and migration compatibility must be part of the release record. No runtime self-update script fetched from the network.
