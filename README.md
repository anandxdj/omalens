# OmaCam

OmaCam is an Omarchy-first Android webcam bridge. This repository is in early implementation: the safety-focused session core and host diagnostics exist, while phone capture, pairing, virtual-camera provisioning, and the Omarchy UI are still under development.

Read [the product documentation](docs/README.md) before contributing. Implementation follows the gated roadmap in [docs/06-development-roadmap.md](docs/06-development-roadmap.md).

## Current developer commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p omacam-daemon -- doctor
cargo run -p omacam-output -- --help
```

`omacam-daemon doctor` is read-only. `omacam-output` currently implements a bounded synthetic-output spike and requires an explicitly supplied existing video device; it never creates or selects a device itself.

## Safety boundary

OmaCam runtime code must not reconfigure networking, USB functions, ADB, default audio, firewall rules, VPNs, unrelated cameras, or desktop security. Camera capture requires explicit phone consent. See [docs/05-security-and-coexistence.md](docs/05-security-and-coexistence.md).

