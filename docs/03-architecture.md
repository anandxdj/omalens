# Architecture and component ownership

Status: recommended design; concrete library versions and proof results remain open in G0.

## Shape of the system

Use a modular Rust control daemon, a small output process, an Android companion, and a thin Omarchy QML plugin. These process boundaries protect the output lifecycle and desktop; they do not justify distributed services, a server database, or cloud infrastructure.

```text
Android companion
  Camera2 -> MediaCodec -> authenticated local media transport
                                  |
                                  v
                       media worker: receive/decode/normalize
                                  |
                           bounded frame channel
                                  v
                     output writer -> OmaCam V4L2 device -> consumer apps
                          |
                          +-> bounded preview delivery -> QML panel

QML panel <-> session D-Bus <-> control daemon <-> authenticated phone control
                                  |
                           owns session policy,
                           trust and worker lifecycle
```

A single decoding stage is the default. The exact placement of preview resizing and frame sharing is decided by the preview feasibility spike. The shell must not decode untrusted network video or receive raw full-resolution frames over D-Bus.

## Ownership boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| QML plugin | Presentation, input, accessibility, view state | Pairing secrets on disk, network media, privileged setup, independent reconnect policy |
| Control daemon | Trusted devices, discovery, capture authorization, session state, commands, diagnostics | Desktop configuration ownership, root authority, global audio policy |
| Media worker | Authenticated media receive, decode, conversion, bounded queues, capture statistics | Trust changes, arbitrary device selection, shell integration |
| Output writer | One validated loopback handle, fixed output mode, neutral-frame watchdog, bounded preview publication | Phone credentials, LAN listeners, control over unrelated video devices |
| Android companion | Its identity key, peer trust, permission flow, camera lifecycle, encoder, session lease, Stop UI | Changing USB functions, root workarounds, hidden APIs, starting capture merely on discovery |
| Package/setup workflow | Declared files, dependencies, scoped device setup, installation receipts | Long-running root daemon, arbitrary scripts from the network, unowned config rewrites |

The daemon is the desktop source of truth. UI commands express intent; success is shown after observed completion. Android is the authority for camera permission and actual capture state. Kernel/output checks are the authority for virtual-camera readiness.

## Recommended stack and alternatives

| Decision | Recommendation | Tradeoff / condition |
| --- | --- | --- |
| Core | Rust and Tokio, small explicit modules | Useful ownership and async cancellation; native media FFI still needs careful isolation |
| Phone | Kotlin, Camera2, MediaCodec | More packaging work than a webpage; supports the required capability and lifecycle model |
| Media transport | Benchmark a maintained WebRTC stack first | Existing congestion and encrypted media machinery; local signaling, identity binding, interoperability, and dependencies still require work |
| Alternative transport | Maintained TLS + media protocol stack only if benchmark evidence warrants it | Potential control over latency; substantially more work for congestion, loss, timing, and security. Do not invent a transport or cipher for MVP |
| Media processing | Evaluate GStreamer with supported Android/Linux codec interoperability | Mature pipeline features; plugin availability, native crashes, copies, and packaging must be measured |
| Output | V4L2loopback initially | Broad compatibility candidate; privileged kernel setup and producer lifecycle require qualification |
| IPC | Session D-Bus for low-volume control/events | Fits desktop integration; high-rate frames use a separate bounded channel |
| Persistence | Versioned, atomic small local records | No server DB needed for a handful of trusted devices; secret storage remains platform-specific |

WebRTC is a starting recommendation, not a finalized protocol. Choose the actual maintained implementation, license set, codec intersection, and certificate binding in G0/G1. Disable external STUN/TURN by default; an unreachable local path must fail visibly. Do not quietly send media or discovery data through a public relay.

## Provider versus transport

Separate capture capability from network reachability. The native provider can use Wi-Fi, hotspot IP, or an already-active USB-tether IP path. A new interface is not necessarily a new camera provider.

Future UVC ingests a local video device; future scrcpy manages a narrowly scoped subprocess. Neither automatically shares the companion's cryptographic identity. Require explicit selection and a separately documented trust model. Serial numbers and friendly names alone cannot unify identities across providers.

Keep the provider interface minimal until a second provider exists. Normalize start/stop, capabilities, settings, and stats. Do not add empty implementations for every future backend.

## Output and preview invariants

- Allocate only an OmaCam-owned loopback device; never hardcode `/dev/video0` or trust a matching label alone.
- For the initial baseline, qualify 1280×720 at 30 fps and a consumer-compatible pixel format. Record the exact format after testing. A mode is not released until both writer and consuming apps accept it.
- Fix the output mode for the writer session. Scale, rotate, and letterbox upstream changes into it. Report source mode and output mode separately.
- Keep the writer attached and producing neutral frames when no phone is active. This supports visibility under `exclusive_caps`; measure idle cost rather than assuming it is free. See [S2](09-decisions-and-sources.md#s2-v4l2loopback).
- The writer rejects old session generations and clears cached captured frames on Stop. A watchdog emits neutral output if worker updates cease.
- Bound the last captured image's lifetime independently of transport error detection: after 500 ms without a fresh acceptable source frame or a healthy controller heartbeat, output becomes neutral. Controller heartbeat timing must satisfy this budget; the phone's separate ten-second authorization lease is not an output-staleness timer.
- Use small bounded queues with a freshness policy: discard obsolete frames instead of accumulating latency. Slow preview or consumer behavior cannot block control handling.
- Closing the preview releases its buffers, not the phone session. UI reopening cannot create a second capture pipeline.
- Preview transfer requires a proven Quickshell-compatible mechanism. If a lightweight native bridge is necessary, review its shell crash risk before adoption; do not smuggle a network decoder into the shell.

A separate output writer improves survival of decoder/worker failures. It does not guarantee continuity if that writer itself dies. Avoid supervisor settings that restart the healthy writer for every worker error. On lost control-daemon heartbeat, the writer clears captured imagery and remains neutral pending a fresh authorized controller.

## Discovery, control, and recovery

Listen for interface changes and optionally mDNS. Discovery records are untrusted hints. QR endpoint candidates must be bounded and tied to the selected pairing session; do not enumerate or probe arbitrary networks.

Use one owner for each active session and serialize conflicting mutations. Tag callbacks, frames, and operations with a session generation so late callbacks cannot resurrect stopped capture. Bound retries, subprocess lifetime, socket buffers, and per-peer state. Use jittered backoff and reset it on successful stable connection; no fleet queue or dead-letter infrastructure is needed.

Transport loss does not automatically imply permission loss. Within the same short-lived armed session, reconnect may proceed. User Stop, Forget, lock, logout, suspend, and lease expiry always disarm it. Start after a full daemon restart requires new consent.

## Local storage and observability

Use the platform's user config, data, state, and runtime directories. Store runtime sockets in the user runtime directory; never in a globally writable predictable location. Use owner-only permissions for trust/config secrets and atomic replacement for records.

Persist identity references, peer key fingerprints, user names, per-device policy, and schema version. Do not persist active capture authorization, frame data, QR tokens, session keys, raw endpoint histories, or pending retries across restart. Prefer Android Keystore and a suitable Linux secret store; document unavailable-store behavior without silently falling back to permissive files.

Expose stage state, reason codes, applied settings, queue depth, frame age, dropped frames, transport RTT, and resource health. Define units and clock domain. Diagnostics are local and redacted; frame or audio dumps are excluded.

## Scaling and external dependencies

Only one active phone is supported. Bound trusted peers, candidate endpoints, pending pairing attempts, and diagnostics storage instead of building infrastructure for hypothetical scale. Main bottlenecks are encoding/decoding, raw-frame copies, Wi-Fi capacity, and phone thermals.

External dependencies include the shell API, Android lifecycle behavior, media libraries, kernel module, PipeWire/portal stack where used, packaging repositories, and signing/distribution. Pin tested versions and record the effects of upgrades; cloud availability is not part of runtime correctness.
