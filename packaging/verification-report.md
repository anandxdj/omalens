# Packaging verification report

Date: 2026-09-09 (Asia/Kolkata)
Base commit: `293f4e2` (the worktree also contains unrelated, pre-existing
changes outside `packaging/`; they were preserved).
Scope: Arch prototype metadata, clean package build, package-content inspection,
systemd unit parsing, and isolated lifecycle checks. No service was enabled or
started, and no V4L2, network, firewall, USB, or live-shell configuration was
changed.

## Dependency metadata

The existing `packaging/arch/PKGBUILD` already has the corrected dependency
set; no source edit was needed:

```text
depends=('gst-plugins-base' 'gst-plugins-bad')
makedepends=('cargo' 'rust')
```

Commands and results:

```text
makepkg --printsrcinfo --dir packaging/arch
```

The generated metadata contains two runtime dependencies (`gst-plugins-base`,
`gst-plugins-bad`), two build dependencies (`cargo`, `rust`), and no
`gst-plugin-openh264` entry. The relevant host checks were:

```text
pacman -Si gst-plugin-openh264
  error: package 'gst-plugin-openh264' was not found
pacman -Qo /usr/lib/gstreamer-1.0/libgstopenh264.so
  /usr/lib/gstreamer-1.0/libgstopenh264.so is owned by gst-plugins-bad 1.28.6-2
```

The installed host versions used for the check were `gst-plugins-base
1.28.6-2`, `gst-plugins-bad 1.28.6-2`, `cargo 1.98.0`, and `rust 1.98.0`.

## Clean package build

Exact command:

```text
makepkg --force --noconfirm --cleanbuild --dir packaging/arch
```

Result: exit 0. The build compiled the locked Rust workspace and produced the
normal and debug package artifacts. `makepkg` printed three `gdb-add-index`
messages saying that no debugging symbols were available for the stripped
release binaries; these were non-fatal tidying messages and did not fail the
build.

Artifacts:

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `packaging/arch/omacam-git-0.1.0-1-x86_64.pkg.tar.zst` | 2,607,698 bytes | `61f26b58d9f22254accefa7b347ea96dd94878212d2692c99c5fa75d50c2f7c1` |
| `packaging/arch/omacam-git-debug-0.1.0-1-x86_64.pkg.tar.zst` | 333,600 bytes | `7776543aebccccdad666e29a21ca861b5f505c88326e8dd7cb1bb75e842acde1` |

The package reports version `0.1.0-1`, runtime dependencies
`gst-plugins-base` and `gst-plugins-bad`, no install script, no signature, and
an unknown packager. The prototype URL, license, maintainer, provenance,
release identity, and signing credentials remain unresolved as required by the
handoff; none were invented.

## Package-content inspection

The main archive contains 22 archive entries: 12 regular files (9 payload
files plus `.BUILDINFO`, `.MTREE`, and `.PKGINFO`) and 10 directories. The
package manager file listing contains 19 payload entries. The nine payload
files are three mode-755 binaries, one mode-644 user unit, two mode-644 docs,
and three mode-644 Omarchy plugin files.

The following checks passed:

```text
pacman -Qlp packaging/arch/omacam-git-0.1.0-1-x86_64.pkg.tar.zst
bsdtar -tf packaging/arch/omacam-git-0.1.0-1-x86_64.pkg.tar.zst
```

- The payload is limited to `/usr` plus pacman metadata; no `/etc`, `/home`,
  `/opt`, or `/var` paths are present.
- No install script is present.
- The packaged unit, example configuration, packaging documentation, and all
  three plugin files are byte-identical to their six source files.
- Extracted `.PKGINFO` has exactly the two corrected runtime dependencies and
  no `gst-plugin-openh264` dependency.

## systemd verification

The direct host parse was run without installing the prototype daemon:

```text
systemd-analyze verify packaging/systemd/omacam.service
```

It returned exit 1 with the expected host-only diagnostic:
`omacam.service: Command /usr/bin/omacam-daemon is not executable: No such
file or directory`.

The coordinator reproduced staged-root verification directly against the
generated package by supplying the user-unit search path inside that root:

```text
SYSTEMD_UNIT_PATH=/usr/lib/systemd/user \
  systemd-analyze --root=packaging/arch/pkg/omacam-git \
  --recursive-errors=no --man=no verify /usr/lib/systemd/user/omacam.service
```

Result: exit 0 with no diagnostics. The explicit root-relative unit search path
is required because this is a user unit rather than a system unit. The unit was
not installed, enabled, started, or supervised.

## Isolated lifecycle evidence

Exact command:

```text
bash packaging/tests/install-lifecycle.sh
```

Result: exit 0. The script exercised 11 assertions across two
`install_owned` cycles (initial install and reinstall as the package's
upgrade/rollback primitive), then simulated removal. It verified executable
and unit modes, absence of a system unit and enablement symlink, preservation of
user configuration, phone trust, desktop identity, and an unrelated sentinel,
and removal of package-owned files.

The harness uses a temporary root and never installs on the host. A rootful
`pacman --root ... -U` experiment was also attempted against a temporary root,
but this session runs as UID 1000 and pacman refused with `you cannot perform
this operation unless you are root`; `sudo -n` is unavailable because the
session has `no new privileges`. Therefore the passing lifecycle evidence is the
repository's isolated temporary-root harness, not a rootful pacman transaction.
There is no genuine versioned upgrade/rollback artifact because the prototype
release identity is intentionally unresolved; a same-version reinstall is the
only permitted package-level primitive at this stage.

## Evidence classes and blockers

- **Build evidence:** clean `makepkg` build passed; normal and debug artifacts
  are recorded above.
- **Metadata/static evidence:** generated `.SRCINFO`, package metadata,
  archive/file/mode inspection, dependency ownership, and staged-root systemd
  parse passed.
- **Isolated lifecycle evidence:** 11 temporary-root assertions passed.
- **Not release evidence:** no host installation, rootful pacman lifecycle,
  service activation, V4L2 provisioning, live-shell test, signing, or release
  provenance was performed.

Remaining blockers are the intentionally unresolved package/release identity,
URL, license, maintainer/provenance, and signing decisions, plus the lack of a
rootful package-manager rehearsal. The generated package files and staging tree
under `packaging/arch/` are ignored build artifacts; the only report source
added by this task is this file.
