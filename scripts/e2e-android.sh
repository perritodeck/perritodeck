#!/usr/bin/env bash
# Runs inside reactivecircus/android-emulator-runner with the emulator booted and the
# host already listening on the runner. Collects screenshots and logcat into e2e-out/.
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/e2e-out"
mkdir -p "$out"

adb logcat -c || true

# Pretend to be a different screen (e.g. a Fire tablet) when the matrix asks for one.
if [ -n "${SCREEN_SIZE:-}" ]; then
  adb shell wm size "$SCREEN_SIZE"
  [ -n "${SCREEN_DENSITY:-}" ] && adb shell wm density "$SCREEN_DENSITY"
  echo "screen: $(adb shell wm size | tr -d '\r' | tail -1), $(adb shell wm density | tr -d '\r' | tail -1)" | tee "$out/screen.txt"
fi

# Keep the app installed after the run so the screenshots can be copied out with run-as.
args=(-Pandroid.testInstrumentationRunnerArguments.pairCode=424242 -Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true)
if [ -n "${HOST_FINGERPRINT:-}" ]; then
  args+=("-Pandroid.testInstrumentationRunnerArguments.hostFingerprint=${HOST_FINGERPRINT}")
fi
(cd "$root/android" && ./gradlew --no-daemon connectedDebugAndroidTest "${args[@]}") 2>&1 | tee "$out/gradle.txt"
status=${PIPESTATUS[0]}

adb logcat -d > "$out/logcat.txt" 2>&1 || true
# Latency and other measurements from the test, as notes on the run summary.
grep -oE "PerritoE2E: latency.*" "$out/logcat.txt" | sed "s/^/::notice title=${E2E_NAME:-E2E}::/" || true
pkg=dev.perritodeck.client.debug
mkdir -p "$out/e2e-screenshots"
for f in $(adb shell run-as "$pkg" ls files/e2e-screenshots 2>/dev/null | tr -d '\r' | grep -E '^[A-Za-z0-9._-]+\.png$'); do
  adb exec-out run-as "$pkg" cat "files/e2e-screenshots/$f" > "$out/e2e-screenshots/$f" 2>/dev/null || true
done
adb exec-out screencap -p > "$out/final-screen.png" 2>/dev/null || true
cp -r "$root/android/app/build/reports/androidTests" "$out/report" 2>/dev/null || true

if [ "$status" -ne 0 ]; then
  # Surface the likely cause as annotations (artifacts aren't always easy to fetch).
  { grep -E "FAILED|AssertionError|Exception|expected|Caused by" "$out/gradle.txt" | head -12
    grep -E "FATAL EXCEPTION|AndroidRuntime|TestRunner: failed|Caused by" "$out/logcat.txt" | head -12; } \
    | sed 's/^/::error::/'
fi

exit $status
