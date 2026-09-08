# OmaCam

OmaCam is an Omarchy-first Android webcam bridge. This repository is in early implementation. The desktop safety core, secure QR pairing, authenticated reconnect and explicit Start control, bounded authenticated media ingress, and one Camera2/MediaCodec 720p30 implementation now exist. The pairing happy path passed on one real phone; the new camera/media path is automated-build and synthetic verified only. Physical control/media trials, persistent output provisioning, preview, and the finished Omarchy UI remain under development.

Read [the product documentation](docs/README.md) before contributing. Implementation follows the gated roadmap in [docs/06-development-roadmap.md](docs/06-development-roadmap.md).

## Current developer commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p omacam-daemon -- doctor
cargo run -p omacam-daemon -- pair status
cargo run -p omacam-output -- --help
```

`omacam-daemon doctor` is read-only. `omacam-output` implements neutral synthetic output plus a development-only framed H.264 ingress and requires an explicitly supplied existing video device; it never creates or selects a device itself. `control serve --request-start --output-device /dev/videoN` requests visible phone consent, then binds the authenticated media channel to the approved peer, connection, session, and generation. This path has not yet been physically qualified.

The current QR pairing trial is documented in [docs/implementation/G3-001-qr-pairing.md](docs/implementation/G3-001-qr-pairing.md). It establishes trust only; it cannot stream camera media yet.

## Safety boundary

OmaCam runtime code must not reconfigure networking, USB functions, ADB, default audio, firewall rules, VPNs, unrelated cameras, or desktop security. Camera capture requires explicit phone consent. See [docs/05-security-and-coexistence.md](docs/05-security-and-coexistence.md).
