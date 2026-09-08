# OmaCam

OmaCam is an Omarchy-first Android webcam bridge. This repository is in early implementation. The desktop safety core, host diagnostics, synthetic output spike, and a secure QR-pairing vertical slice now exist. Pairing has built successfully but still needs its first real-phone trial; camera capture, authenticated media, persistent output provisioning, and the finished Omarchy UI remain under development.

Read [the product documentation](docs/README.md) before contributing. Implementation follows the gated roadmap in [docs/06-development-roadmap.md](docs/06-development-roadmap.md).

## Current developer commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p omacam-daemon -- doctor
cargo run -p omacam-daemon -- pair status
cargo run -p omacam-output -- --help
```

`omacam-daemon doctor` is read-only. `omacam-output` currently implements a bounded synthetic-output spike and requires an explicitly supplied existing video device; it never creates or selects a device itself.

The current QR pairing trial is documented in [docs/implementation/G3-001-qr-pairing.md](docs/implementation/G3-001-qr-pairing.md). It establishes trust only; it cannot stream camera media yet.

## Safety boundary

OmaCam runtime code must not reconfigure networking, USB functions, ADB, default audio, firewall rules, VPNs, unrelated cameras, or desktop security. Camera capture requires explicit phone consent. See [docs/05-security-and-coexistence.md](docs/05-security-and-coexistence.md).
