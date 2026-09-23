#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$PROJECT_ROOT"

echo "==> Checking patch hygiene"
git diff --check

echo "==> Checking Rust formatting, compilation, tests, and lints"
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --release --locked

if command -v qmllint >/dev/null 2>&1; then
    echo "==> Checking QML"
    qmllint -I /usr/share/omarchy/shell omarchy-plugin/*.qml
else
    echo "ERROR: qmllint is required for release verification" >&2
    exit 1
fi

ANDROID_SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
if [[ -z "$ANDROID_SDK" || ! -d "$ANDROID_SDK" ]]; then
    echo "ERROR: set ANDROID_HOME or ANDROID_SDK_ROOT to an installed Android SDK" >&2
    exit 1
fi
if [[ -z "${JAVA_HOME:-}" || ! -x "$JAVA_HOME/bin/java" ]]; then
    echo "ERROR: set JAVA_HOME to a JDK 17 installation" >&2
    exit 1
fi

echo "==> Checking Android JVM tests, APKs, and lint"
./android/gradlew -p android --no-daemon --rerun-tasks \
    testDebugUnitTest assembleDebug assembleDebugAndroidTest assembleRelease \
    lintDebug lintRelease

echo "==> Checking package lifecycle"
bash packaging/tests/install-lifecycle.sh

echo "Automated release-candidate verification passed."
echo "Connected-device, live-shell, consumer, performance, and signing gates remain manual."
