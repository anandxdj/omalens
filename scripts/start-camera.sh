#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

DAEMON="$PROJECT_ROOT/target/release/omacam-daemon"
if [[ ! -x "$DAEMON" ]]; then
    echo "Error: release binaries are missing. Run cargo build --release --locked --workspace." >&2
    exit 1
fi

RUNTIME_ROOT=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
PREVIEW_DIR="$RUNTIME_ROOT/omacam"
PREVIEW_SOCKET="$PREVIEW_DIR/preview.sock"
OUTPUT_DEVICE=${OMACAM_OUTPUT_DEVICE:-/dev/video42}

# 1. Detect LAN IP
HOST_IP=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{print $7}' || true)
if [[ -z "$HOST_IP" ]]; then
    HOST_IP=$(ip -4 -brief addr show up | awk '$1 !~ /^lo/ {print $3}' | cut -d/ -f1 | head -n1)
fi

echo "================================================="
echo "   OmaCam Wireless Camera Streamer               "
echo "   Host IP: $HOST_IP                             "
echo "================================================="

# 2. Require an intentionally provisioned output device. This launcher never
# changes kernel modules or chooses an unrelated camera on the user's behalf.
if [[ ! -c "$OUTPUT_DEVICE" ]]; then
    echo "Error: $OUTPUT_DEVICE is not an existing character device." >&2
    echo "Provision and verify a dedicated OmaCam v4l2loopback node first, or set OMACAM_OUTPUT_DEVICE." >&2
    exit 1
fi

# 3. Clean stale preview socket
mkdir -p "$PREVIEW_DIR"
chmod 700 "$PREVIEW_DIR"
rm -f "$PREVIEW_SOCKET"

# 4. Check if phone is trusted; if not, do automatic pairing
TRUST_STATUS=$($DAEMON pair status 2>&1 || true)
if [[ "${1:-}" == "--pair" || "${1:-}" == "--fresh" ]] || grep -q "No phone is trusted" <<<"$TRUST_STATUS"; then
    if [[ "${1:-}" == "--fresh" ]]; then
        $DAEMON pair forget
    fi
    echo "==> Generating pairing QR code..."
    rm -f /tmp/omacam-qr.svg /tmp/omacam-qr.png
    $DAEMON pair serve --endpoint "${HOST_IP}:47123" --qr /tmp/omacam-qr.svg &
    PAIR_PID=$!
    sleep 1
    if [ -f /tmp/omacam-qr.svg ]; then
        rsvg-convert -w 600 -h 600 /tmp/omacam-qr.svg -o /tmp/omacam-qr.png 2>/dev/null || true
        if [ -f /tmp/omacam-qr.png ]; then
            WAYLAND_DISPLAY=wayland-1 DISPLAY=:0 imv /tmp/omacam-qr.png &
            IMV_PID=$!
        fi
    fi
    echo "==> Scan the QR code with OmaCam on your phone..."
    wait $PAIR_PID
    kill "${IMV_PID:-0}" 2>/dev/null || true
    echo "==> Pairing complete! Proceeding to camera stream..."
    sleep 1
fi

# 5. Check if omacam-daemon service is already running
DAEMON_STARTED=0
if ! pgrep -f "[o]macam-daemon service" >/dev/null 2>&1; then
    echo "==> Starting OmaCam background service..."
    $DAEMON service \
        --listen "${HOST_IP}:47123" \
        --output-device "$OUTPUT_DEVICE" \
        --preview-socket "$PREVIEW_SOCKET" &
    DAEMON_PID=$!
    DAEMON_STARTED=1
    sleep 1
else
    echo "==> OmaCam service is already running."
fi

# Broadcast mDNS advertisement via avahi-publish
AVAHI_PID=""
if command -v avahi-publish >/dev/null 2>&1; then
    CERT_DIGEST=$(python3 -c "
import json, hashlib, base64
with open('$HOME/.local/share/omacam/desktop-identity.json') as f:
    data = json.load(f)
print(base64.urlsafe_b64encode(hashlib.sha256(bytes(data['certificate_der'])).digest()).decode().rstrip('='))
" 2>/dev/null || true)
    if [ -n "$CERT_DIGEST" ]; then
        SUFFIX="${CERT_DIGEST:0:12}"
        avahi-publish -s "OmaCam-${SUFFIX}" _omacam._tcp 47123 "v=1" "cert=${CERT_DIGEST}" "role=desktop-control" >/dev/null 2>&1 &
        AVAHI_PID=$!
    fi
fi

trap 'if [ "$DAEMON_STARTED" -eq 1 ]; then echo "==> Stopping service..."; kill "${DAEMON_PID:-0}" 2>/dev/null || true; fi; kill "${AVAHI_PID:-0}" 2>/dev/null || true' EXIT

# 5. Wait for phone connection
echo "==> Waiting for phone connection..."
echo "    (If your phone shows 'FIND TRUSTED LAPTOP', tap it now)"

PHONE_CONNECTED=0
for i in {1..600}; do
    SNAP=$($DAEMON ipc snapshot 2>/dev/null || true)
    if echo "$SNAP" | grep -q '"connection":"online"'; then
        PHONE_CONNECTED=1
        echo "==> Phone connected!"
        break
    fi
    sleep 1
done

if [ "$PHONE_CONNECTED" -eq 0 ]; then
    echo "Error: Phone did not connect within 10 minutes."
    exit 1
fi

# 6. Request camera start
OP_ID="op_$(date +%s)"
echo "==> Prompting phone for camera capture..."
echo "    ==> Tap 'SHARE CAMERA' on your phone <=="
$DAEMON ipc start "$OP_ID"

# 7. Wait for streaming
STREAM_READY=0
for i in {1..30}; do
    SNAP=$($DAEMON ipc snapshot 2>/dev/null || true)
    if echo "$SNAP" | grep -q '"capture":"streaming"'; then
        STREAM_READY=1
        echo "==> Live camera stream active!"
        break
    fi
    sleep 1
done

# 8. Launch viewer
echo "==> Opening camera preview window..."
ffplay -f v4l2 -video_size 1280x720 -framerate 30 -window_title "OmaCam Live Camera" "$OUTPUT_DEVICE"
