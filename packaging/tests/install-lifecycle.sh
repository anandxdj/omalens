#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
stage_root=$(mktemp -d)
trap 'rm -rf -- "$stage_root"' EXIT

install_owned() {
  install -Dm755 "$repo_root/target/release/omacam-daemon" "$stage_root/usr/bin/omacam-daemon"
  install -Dm755 "$repo_root/target/release/omacam-output" "$stage_root/usr/bin/omacam-output"
  install -Dm755 "$repo_root/target/release/omacam-preview" "$stage_root/usr/bin/omacam-preview"
  install -Dm644 "$repo_root/packaging/systemd/omacam.service" "$stage_root/usr/lib/systemd/user/omacam.service"
  install -Dm644 "$repo_root/packaging/systemd/service.conf.example" "$stage_root/usr/share/doc/omacam/service.conf.example"
  install -Dm644 "$repo_root/packaging/README.md" "$stage_root/usr/share/doc/omacam/packaging.md"
  mkdir -p "$stage_root/usr/share/omacam/omarchy-plugin"
  cp -a "$repo_root/omarchy-plugin/." "$stage_root/usr/share/omacam/omarchy-plugin/"
}

install_owned
test "$(stat -c %a "$stage_root/usr/bin/omacam-daemon")" = 755
test "$(stat -c %a "$stage_root/usr/lib/systemd/user/omacam.service")" = 644
test ! -e "$stage_root/etc/systemd/system/omacam.service"
test ! -e "$stage_root/usr/lib/systemd/user/graphical-session.target.wants/omacam.service"

mkdir -p "$stage_root/home/test/.config/omacam" "$stage_root/home/test/.local/state/omacam" "$stage_root/opt/unrelated"
printf 'user-owned\n' > "$stage_root/home/test/.config/omacam/service.conf"
printf 'trust\n' > "$stage_root/home/test/.local/state/omacam/trusted-phone.json"
printf 'identity\n' > "$stage_root/home/test/.local/state/omacam/desktop-identity.json"
printf 'keep\n' > "$stage_root/opt/unrelated/sentinel"

# Reinstall is the package's upgrade/rollback primitive and must not touch user data.
install_owned
test "$(cat "$stage_root/home/test/.local/state/omacam/trusted-phone.json")" = trust
test "$(cat "$stage_root/home/test/.local/state/omacam/desktop-identity.json")" = identity

rm -f -- "$stage_root/usr/bin/omacam-daemon" "$stage_root/usr/bin/omacam-output" "$stage_root/usr/bin/omacam-preview"
rm -f -- "$stage_root/usr/lib/systemd/user/omacam.service" "$stage_root/usr/share/doc/omacam/service.conf.example" "$stage_root/usr/share/doc/omacam/packaging.md"
rm -rf -- "$stage_root/usr/share/omacam/omarchy-plugin"

test "$(cat "$stage_root/home/test/.config/omacam/service.conf")" = user-owned
test "$(cat "$stage_root/home/test/.local/state/omacam/trusted-phone.json")" = trust
test "$(cat "$stage_root/home/test/.local/state/omacam/desktop-identity.json")" = identity
test "$(cat "$stage_root/opt/unrelated/sentinel")" = keep
test ! -e "$stage_root/usr/bin/omacam-daemon"
