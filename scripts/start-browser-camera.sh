#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$PROJECT_ROOT"

DAEMON=${OMACAM_DAEMON:-$PROJECT_ROOT/target/release/omacam-daemon}
OUTPUT_DEVICE=${OMACAM_OUTPUT_DEVICE:-/dev/video42}
HTTPS_PORT=${OMACAM_BROWSER_PORT:-8443}
RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/omacam
QR_SVG=$RUNTIME_DIR/browser-camera.svg
QR_PNG=$RUNTIME_DIR/browser-camera.png

for program in ip imv rsvg-convert; do
    if ! command -v "$program" >/dev/null 2>&1; then
        echo "Missing required program: $program" >&2
        exit 1
    fi
done
if [[ ! -x "$DAEMON" ]]; then
    echo "Release binary is missing; run cargo build --release --locked --workspace." >&2
    exit 1
fi
if [[ ! -c "$OUTPUT_DEVICE" ]]; then
    echo "$OUTPUT_DEVICE is not an existing virtual camera." >&2
    exit 1
fi
if pgrep -f '[o]macam-daemon browser serve' >/dev/null 2>&1; then
    echo "An OmaCam browser session is already running." >&2
    exit 1
fi

HOST_IP=$(ip -4 route get 192.0.2.1 2>/dev/null | awk '{print $7; exit}')
if [[ -z "$HOST_IP" ]]; then
    echo "No active LAN address was found." >&2
    exit 1
fi

mkdir -p "$RUNTIME_DIR"
chmod 700 "$RUNTIME_DIR"
rm -f "$QR_SVG" "$QR_PNG"

cleanup() {
    kill "${QR_PID:-0}" 2>/dev/null || true
    kill "${DAEMON_PID:-0}" 2>/dev/null || true
    rm -f "$QR_SVG" "$QR_PNG"
}
trap cleanup EXIT INT TERM

"$DAEMON" browser serve \
    --endpoint "$HOST_IP:$HTTPS_PORT" \
    --output-device "$OUTPUT_DEVICE" \
    --qr "$QR_SVG" &
DAEMON_PID=$!

for _ in {1..50}; do
    [[ -s "$QR_SVG" ]] && break
    kill -0 "$DAEMON_PID" 2>/dev/null || wait "$DAEMON_PID"
    sleep 0.1
done
if [[ ! -s "$QR_SVG" ]]; then
    echo "OmaCam did not create its QR code." >&2
    exit 1
fi

rsvg-convert -w 720 -h 720 "$QR_SVG" -o "$QR_PNG"
imv "$QR_PNG" &
QR_PID=$!

echo "Scan the QR, accept the local certificate once, then allow the camera."
echo "OmaCam uses local HTTPS and WebRTC/H.264 only; no domain or cloud relay."
echo "Firewall: allow TCP and UDP port $HTTPS_PORT from this phone/LAN."
echo "Select $OUTPUT_DEVICE in OBS, Zoom, Meet, or Discord."

wait "$DAEMON_PID"
