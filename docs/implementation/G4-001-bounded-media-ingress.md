# G4-001 — Session-bound desktop media ingress

Status: desktop contract, validation, and synthetic decode/neutral topology implemented and automated-verified. Authenticated network receive and phone capture are now implemented in [G4-002](G4-002-authenticated-start-camera-media.md), but remain physically untested; real V4L2 output, preview, and physical Stop timing remain pending. This is not a completed G4 gate.

Requirements: R05, R06, R09–R11, R13, R15, R20. Gate: partial G4, with reusable G1/G2 contract work.

## Objective and scope

Turn the preliminary uncommitted `omacam-output --h264-stdin` experiment into a bounded production-directed ingress boundary before exposing any network media. The raw mode was replaced because an arbitrary unframed byte stream could not prove which peer, authenticated connection, media session, or capture generation produced a frame and could not reject late frames after Stop.

This package intentionally did not start Android camera capture or claim authenticated network media. It established the exact local daemon-to-output contract now fed by G4-002 only after authenticated control and explicit phone consent.

## Implemented contract

`omacam-core::media` defines protocol version 1. Each record has a fixed 104-byte big-endian header followed by at most one bounded payload:

| Offset | Size | Field |
| --- | ---: | --- |
| 0 | 8 | magic `OMACAMM1` |
| 8 | 1 | version `1` |
| 9 | 1 | kind: `1` H.264 access unit, `2` terminal Stop |
| 10 | 2 | flags; bit 0 is key-frame and all other bits are rejected |
| 12 | 32 | SHA-256 identity of the authenticated phone key |
| 44 | 16 | fresh authenticated control-connection identifier |
| 60 | 16 | unpredictable media-session identifier |
| 76 | 8 | capture generation |
| 84 | 8 | sequence, starting at zero and contiguous |
| 92 | 8 | monotonic presentation time in microseconds; zero for Stop |
| 100 | 4 | payload length |

H.264 access units must be non-empty and no larger than 1 MiB. A record is rejected before payload allocation when its magic, version, flags, peer, connection, session, generation, sequence, timestamp, or length is invalid. A partial header or payload fails closed. Stop has no flags, timestamp, or payload; it invalidates the validator before any later record can reach the decoder.

The binding fields are context, not reusable credentials. The next network package must derive them from a freshly authenticated TLS control connection and an explicitly consented capture generation. It must not accept these fields as proof on an unauthenticated socket.

## Output worker changes

The raw `--h264-stdin` option was removed. The development ingress now requires all binding fields explicitly:

```text
omacam-output --device /dev/videoN --framed-h264-stdin \
  --peer <64_HEX> --connection <32_HEX> --session <32_HEX> --generation <U64>
```

Only validated H.264 access-unit payloads are written to the isolated GStreamer decoder. The selected codec contract is currently H.264 constrained-baseline, byte-stream access-unit alignment, 1280×720 at 30 fps. Decode output is converted, scaled, and rate-normalized to I420 1280×720 at 30 fps before the compositor produces fixed YUY2 1280×720 at 30 fps for V4L2.

The decoder queue is limited to two buffers and is downstream-leaky to prefer freshness. A permanent black live source remains below the decoded source. The decoded compositor pad stops repeating its last buffer after 400 ms, leaving margin under the 500 ms privacy target. EOF, malformed input, binding failure, and Stop close only the decoder input; the neutral pipeline remains alive. This timing/topology is automated and synthetic evidence only until measured with recognizable imagery through a real V4L2 consumer.

## Automated evidence from 2026-09-08

- `cargo test --workspace`: 47 tests pass (36 core, 6 daemon, 5 output).
- Media tests cover malformed/truncated headers and payloads, the 1 MiB allocation bound, wrong peer, wrong control connection, wrong media session, stale generation, gaps/replay, timestamp regression, terminal Stop, late post-Stop data, and strict binding parsing.
- Output tests prove that framing is stripped before decode, terminal Stop prevents later payload forwarding, and stale-generation bytes never reach the decoder.
- `cargo clippy --workspace --all-targets -- -D warnings`: passes.
- `omacam-output --probe`: neutral and H.264 live-input dependencies report ready.
- The exact compositor/decode/normalize topology accepted a locally generated constrained-baseline 1280×720@30 H.264 stream into `fakesink`. After source EOF it remained running on the live neutral branch until the five-second harness timeout, as intended.
- An empty media source also left the neutral topology running until the three-second harness timeout.
- Android `assembleDebug`, `lintDebug`, and `testDebugUnitTest` succeed. The unit-test task has no JVM sources.
- `git diff --check`: passes after the package.

## Security and coexistence review

- No camera, microphone, network listener, cloud service, USB function, route, DNS, firewall, VPN, shared video device, module, desktop configuration, or audio setting was changed.
- The decoder remains outside the daemon and shell. External input controls no command string or device selection.
- This package does not authorize capture. Pairing and authenticated reconnect behavior remain unable to start media.
- Binding values are visible command arguments in this development interface, so they must contain identifiers only, never media secrets. Production process setup should pass the validated stream through owned file descriptors and avoid exposing even identifiers unnecessarily.

## Still required

1. Physically verify the G4-002 Start, Camera2/MediaCodec, authenticated media, Stop, and lease implementation on the OnePlus.
2. Exercise recognizable imagery through a real OmaCam-owned V4L2 device and measure neutral replacement within 500 ms for phone Stop, desktop Stop, link loss, malformed media, and lease expiry.
3. Add the shared preview path without a second decoder or phone stream.

Physical verification: none for this package. No real phone camera, output device, meeting application, or preview was used.
