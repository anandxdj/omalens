# Packaging and installation status

The files in this directory build the OmaCam 0.1 release candidate from the
current repository checkout. The package has an MIT license and canonical
project URL, but it must not be published until a version tag is created, a
source archive checksum is pinned, and the package and Android APK are signed
by keys controlled by the release owner.

The package installs three fixed-purpose binaries, the user unit, one example
configuration, and the OmaCam shell plugin under `/usr/share/omacam`. It has no
install script and does not enable or start the service. It does not provision
V4L2 devices or alter the firewall, networking, USB, PipeWire, or the live
Omarchy configuration.

After a package is installed, the user must intentionally:

1. Provision a dedicated, OmaCam-owned `v4l2loopback` device. Do not select an
   existing camera by label or path alone.
2. Copy `service.conf.example` to `~/.config/omacam/service.conf`, replace both
   documentation-only values with an existing reachable LAN address and the
   verified dedicated output node, and restrict the file to the user.
3. Install/activate the shell plugin through the supported Omarchy workflow.
4. Run `systemctl --user enable --now omacam.service` only after reviewing the
   configuration.

Uninstall removes only package-owned `/usr` artifacts. User configuration under
`~/.config/omacam` and packaged-service state under `~/.local/state/omacam` are
retained so an ordinary package removal cannot silently destroy device trust.
The user may remove them separately after stopping the service and deciding to
forget all peers. The packaged unit passes its `StateDirectory` paths
explicitly, so its trust and desktop identity remain writable while
`ProtectHome=read-only` is active.

`packaging/tests/install-lifecycle.sh` stages the package-owned layout in a
temporary root, checks mode/ownership scope and unit behavior, simulates an
upgrade and rollback, and proves that uninstall leaves user data and unrelated
files intact. It never installs on the host.
