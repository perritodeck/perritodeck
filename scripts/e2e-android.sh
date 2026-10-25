#!/usr/bin/env bash
# Runs inside reactivecircus/android-emulator-runner with the emulator booted and the
# host already listening on the runner. Collects screenshots and logcat into e2e-out/.
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/e2e-out"
mkdir -p "$out"

adb logcat -c || true

# The test's shell commands run as whoever adbd runs as, and on Android 7 the shell user
# can't switch Wi-Fi off (svc wifi: no CHANGE_WIFI_STATE). The emulator images allow root.
adb root || true
adb wait-for-device
echo "adb shell uid: $(adb shell id -u | tr -d '\r')" | tee "$out/adb-uid.txt"

# Pretend to be a different screen (e.g. a Fire tablet) when the matrix asks for one.
if [ -n "${SCREEN_SIZE:-}" ]; then
  adb shell wm size "$SCREEN_SIZE"
  [ -n "${SCREEN_DENSITY:-}" ] && adb shell wm density "$SCREEN_DENSITY"
  echo "screen: $(adb shell wm size | tr -d '\r' | tail -1), $(adb shell wm density | tr -d '\r' | tail -1)" | tee "$out/screen.txt"

  # The API 25 image's Launcher3 crashes when the screen changes size (an NPE as it
  # relaunches), and its "Launcher3 has stopped" dialog then stays on top of everything:
  # the tests' taps can't reach it, and the test runner can't bring up its own activity
  # to close the app's, so every test after the first fails. Close it once it's up.
  # (Back, if closing system dialogs doesn't do it, goes to the focused dialog.)
  # (Not `dumpsys | grep -q`: grep quitting early fails the pipe under pipefail.)
  # The focused window counts: a closed dialog's window can stay in the list for a while.
  crash_dialog() { adb shell dumpsys window windows | grep "mCurrentFocus=" | grep -o "Application Error: [^ }]*" | head -1; }
  closed=""
  for _ in $(seq 1 10); do
    sleep 1
    dialog="$(crash_dialog)"
    if [ -n "$dialog" ]; then
      echo "closing a crash dialog: $dialog"
      if [ -z "$closed" ]; then
        adb shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS > /dev/null
        closed=1
      else
        adb shell input keyevent KEYCODE_BACK
      fi
    fi
  done
  dialog="$(crash_dialog)"
  if [ -n "$dialog" ]; then
    echo "::warning::a crash dialog is still on screen: $dialog"
  fi
fi

# Keep the app installed after the run so the screenshots can be copied out with run-as.
args=(-Pandroid.testInstrumentationRunnerArguments.pairCode=424242 -Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true)
if [ -n "${HOST_FINGERPRINT:-}" ]; then
  args+=("-Pandroid.testInstrumentationRunnerArguments.hostFingerprint=${HOST_FINGERPRINT}")
fi
# Not piped through tee: if Gradle hangs, the timeout must be able to end it (a pipe
# would stay open in its child processes).
(cd "$root/android" && timeout --kill-after=30 1500 ./gradlew --no-daemon connectedDebugAndroidTest "${args[@]}") > "$out/gradle.txt" 2>&1
status=$?
cat "$out/gradle.txt"

timeout 60 adb logcat -d > "$out/logcat.txt" 2>&1 || true
# Latency and other measurements from the test, as notes on the run summary.
grep -oE "PerritoE2E: latency.*" "$out/logcat.txt" | sed "s/^/::notice title=${E2E_NAME:-E2E}::/" || true
pkg=dev.perritodeck.client.debug
mkdir -p "$out/e2e-screenshots"
for f in $(timeout 30 adb shell run-as "$pkg" ls files/e2e-screenshots 2>/dev/null | tr -d '\r' | grep -E '^[A-Za-z0-9._-]+\.png$'); do
  timeout 30 adb exec-out run-as "$pkg" cat "files/e2e-screenshots/$f" > "$out/e2e-screenshots/$f" 2>/dev/null || true
done
timeout 30 adb exec-out screencap -p > "$out/final-screen.png" 2>/dev/null || true
cp -r "$root/android/app/build/reports/androidTests" "$out/report" 2>/dev/null || true

if [ "$status" -ne 0 ]; then
  # Surface the likely cause as annotations (artifacts aren't always easy to fetch).
  { grep -E "FAILED|AssertionError|Exception|expected|Caused by" "$out/gradle.txt" | head -12
    grep -E "FATAL EXCEPTION|AndroidRuntime|TestRunner: failed|Caused by" "$out/logcat.txt" | head -12; } \
    | sed 's/^/::error::/'
fi

exit $status
