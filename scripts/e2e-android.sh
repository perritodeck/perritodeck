#!/usr/bin/env bash
# Runs inside reactivecircus/android-emulator-runner with the emulator booted and the
# host already listening on the runner. Collects screenshots and logcat into e2e-out/.
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/e2e-out"
mkdir -p "$out"

adb logcat -c || true

(cd "$root/android" && ./gradlew --no-daemon connectedDebugAndroidTest)
status=$?

adb logcat -d > "$out/logcat.txt" 2>&1 || true
adb pull /sdcard/Android/data/dev.perritodeck.client.debug/files/e2e-screenshots "$out/" > /dev/null 2>&1 || true
adb exec-out screencap -p > "$out/final-screen.png" 2>/dev/null || true
cp -r "$root/android/app/build/reports/androidTests" "$out/report" 2>/dev/null || true

exit $status
