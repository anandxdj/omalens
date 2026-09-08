# Contracts and lifecycle semantics

These are behavioral contracts, not production schemas or code. Before implementation, G1 must produce versioned wire and D-Bus specifications with field types, encoding, maximum lengths, validation, and example request/result fixtures. Do not infer security from the illustrative operation names below.

## Independent state axes

Do not collapse pairing, transport, capture, and output into a single Connected boolean.

| Axis | States | Authority |
| --- | --- | --- |
| Trust | Unpaired, PairingPending, Trusted, Revoked | Local trust store plus authenticated peer proof |
| Connection | Offline, Discovered, Authenticating, Online, Recovering | Connection manager |
| Capture | Idle, AwaitingConsent, Starting, Streaming, Switching, Stopping, Failed | Daemon intent reconciled with phone acknowledgements |
| Output | Missing, Preparing, ReadyNeutral, ReadyLive, Failed | Output writer and device checks |

The UI may say “Ready for apps” only when output is ready; it may say “Live” only when fresh frames from the current authorized session reach that output. Preview alone is insufficient.

## Capture transitions

| Event / precondition | Required transition | Required side effects |
| --- | --- | --- |
| Start requested, trusted peer online | Idle → AwaitingConsent | Send request for phone consent; keep output neutral |
| Phone approves and OS allows capture | AwaitingConsent → Starting → Streaming | Create new session generation, acquire camera, negotiate valid mode, verify first output frame |
| Permission denied or camera busy | AwaitingConsent/Starting → Failed | Release partial resources; typed error; no automatic permission loop |
| Link lost during authorized capture | Streaming → Recovering connection state | Clear captured output within stale-frame budget; attempt authenticated reconnection |
| Same peer returns within armed lease | Recovering → Online; capture resumes | Fresh generation for pipeline buffers; preserve current authorization only within its lease |
| Lease expires | Any armed capture → Stopping → Idle | Phone releases capture; no unattended restart |
| Stop / Forget / lock / suspend / logout | Any capture state → Stopping → Idle | Invalidate authorization first, then cancel retries and operations, clear frames, release capture and torch |
| Lens change supported | Streaming → Switching → Streaming | Serialize change, neutral output during gap, publish new capabilities and actual settings |
| Lens change fails | Switching → prior Streaming or Failed | Restore prior usable mode if possible; otherwise remain neutral |
| Control process restarts | Capture becomes Idle | No persisted permission to resume capture; reauthenticate and request Start |

Recovering belongs to the connection axis; capture remains logically armed only until the lease expires. Late success callbacks for cancelled generations must be ignored. Terminal Stop takes priority over all other queued operations.

## Session policy defaults

Proposed baseline values to validate in G1; altering them requires updating tests and this document.

- Pairing invitation lifetime: 120 seconds, measured by the issuer's monotonic timer; at least 128 bits of cryptographically random invitation entropy.
- One pending invitation and one pending approval on a desktop at a time. Atomic consumption prevents two scanners redeeming the same invitation. A failed or cancelled claimed invitation requires a new QR.
- A capture lease is refreshed over authenticated control every 2 seconds and expires after 10 seconds without a valid refresh. No remote timestamp can extend it.
- A short connection outage can resume only inside that lease. A longer outage requires explicit phone consent again.
- Ask is the default. Auto-connect authenticates and establishes control; it does not create new capture authorization. Ignore suppresses unsolicited connection attempts until the user selects the phone.
- Proposed bounds: 16 trusted peers, 8 endpoint candidates per invitation, 64 KiB maximum control message, 32 pending control operations. Reject overflow safely. G1 may tighten these after serialization is chosen.
- Pairing attempts use per-source and global rate limits, initially 5 failed attempts per minute per source and 20 globally, without blocking an established Stop command. Enforce bounded unauthenticated work before expensive allocation.

## Identity and pairing records

| Record | Required concepts |
| --- | --- |
| Pairing invitation | Invitation version, session identifier, issuer identity fingerprint/public-key binding, ephemeral secret, bounded endpoints, lifetime hint |
| Trusted peer | Stable public-key-based identity, display name, trust creation time, policy, key-store reference, record schema version |
| Session | Unpredictable identifier, local generation, authenticated peer identities, selected protocol version, negotiated capabilities, authorization lease |
| Operation | Request identifier, session generation where applicable, deadline, typed parameters, result or typed error |

Display names are untrusted text with strict length and rendering limits. IP addresses, mDNS names, ADB serials, USB labels, and camera labels are not cryptographic identities.

## Capability contract

Each camera has a provider-scoped opaque identifier, a friendly label, facing, logical/physical relationship where known, label confidence, and a revision. Camera identifiers may change after updates; refresh capabilities instead of treating stale IDs as permanent.

Represent **valid mode tuples**, not independently combinable arrays. Each supported mode describes camera selection, width, height, FPS or valid range, pixel/encoder format, codec/profile constraints, relevant dynamic-range restrictions, and incompatible controls. The phone must validate the requested capture session at runtime even after capability discovery.

Each control describes supported state, availability reason, units, minimum/maximum/step or enum values, current and default value, auto/manual dependencies, and whether a restart is needed. Exposure compensation is not manual shutter time; zoom ratio is not a physical-lens guarantee. Manual focus may become available only after disabling autofocus.

Illustrative valid capability: “Rear camera, 1280×720 at 30 fps, zoom 1×–4×, exposure compensation −2 to +2 EV in 1/3 EV steps.” This does not imply that 1920×1080 at 60 fps works or that a separate telephoto camera can be opened.

Requests carry the capability revision. A stale revision produces a refresh-required error rather than applying values against a new camera. Return actual applied settings, including clamping only where the client explicitly allows it.

## Local control API

Use a versioned D-Bus interface and bounded snapshots/events. The final namespace follows the resolved product name. Proposed operations:

| Operation | Preconditions and result |
| --- | --- |
| GetSnapshot | Returns all state axes, capabilities revision, applied settings, output status, protocol/API versions |
| BeginPairing / CancelPairing | Create or invalidate the sole active invitation; never log the secret |
| ApprovePairing / RejectPairing | Refer to the exact pending authenticated transcript and peer, not just a display name |
| RequestStart | Trusted selected peer, no competing session; returns an operation ID, not premature success |
| Stop | Idempotent; effective in every state; invalidates current session before resource cleanup |
| SetCamera / SetControl / SetQuality | Current session, current capability revision, validated value; asynchronous applied result |
| ForgetPeer | Revoke first, stop applicable session, then remove persistent credentials |
| SetPeerPolicy | Explicit user intent; cannot itself start camera capture |
| RunDiagnostics | Read-only bounded checks; no hidden repair or privilege escalation |

Events carry a monotonically increasing local revision and operation/session identity. After missed events or reconnection, clients fetch a fresh snapshot. No secret pairing payload in broadcasts visible to unrelated consumers; return sensitive data only through the intended local call path with documented session-bus limits.

The session bus is not a security boundary against malicious processes running as the same desktop user. Protect against other users and accidental misuse; document that a compromised user session can control user-owned resources. Phone consent remains independently enforced.

## Error and retry contract

Errors include a stable code, affected stage, retryability, operation ID, sanitized user message, and optional supported alternatives. Initial categories: PermissionDenied, ConsentRequired, CameraBusy, Unreachable, AuthenticationFailed, InvitationExpired, ReplayRejected, IncompatibleVersion, UnsupportedMode, StaleCapabilities, OutputUnavailable, ResourceLimit, Cancelled, and InternalFailure.

Do not retry permission denial, revocation, malformed requests, or incompatible major versions. Retry transient connectivity with bounded jittered backoff only within the session policy. Deduplicate mutating requests by request ID and session generation for a bounded period; repeated Stop must remain safe after entries expire.

## Protocol evolution

Handshake negotiates explicit supported protocol ranges, feature flags, and required features. Reject an unsupported major version before camera activation. Minor additions must be optional; unknown security-critical fields or required features fail closed. Never downgrade to unauthenticated or plaintext communication.

Proposed initial compatibility policy: support the current major protocol with additive minor versions; no previous-major promise for the first release. Test current desktop with the previous supported companion release and the reverse when such versions exist. Persisted schema migrations are separate from wire compatibility and require rollback analysis.

## Media timing and output

Preserve capture timestamps where available, including their clock domain. Use monotonic clocks and a documented offset/drift mapping for presentation; never subtract unrelated clocks directly. Reject frames from old session generations and bound frame age and dimensions before allocation.

Output timestamps remain monotonic across source switching. Neutral frames use the output clock. A future audio milestone must specify synchronization, discontinuity handling, drift correction, and silence-on-disconnect; its implementation must not be inferred from the MVP video contract.
