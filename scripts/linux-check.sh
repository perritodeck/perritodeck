#!/usr/bin/env bash
# The host and desktop app checks on Linux, against a COMMITTED tree: what CI's Linux jobs
# run, for when CI can't (it is blocked until the repository goes public; see
# docs/DECISIONS.md, 2026-10-15). Works on Linux and in WSL.
#
#   bash scripts/linux-check.sh                                   (Linux)
#   wsl.exe -d Ubuntu -- bash /mnt/<drive>/<path>/scripts/linux-check.sh   (Windows)
#
# What it does:
#   1. git archive <REF> of this checkout into a fresh $PD/src. The working tree, and its
#      host/target and host/app/src-tauri/target, are never touched.
#   2. host/:               cargo fmt --all --check; cargo clippy --all-targets --locked -- -D warnings;
#                           cargo test --locked (all crates; --no-fail-fast so every failure shows)
#   3. protocol/:           python3 protocol/tools/validate.py protocol
#   4. host/app:            npm ci; npm run build (src-tauri embeds ../dist)
#      host/app/src-tauri:  cargo fmt --check; cargo clippy --locked --all-targets -- -D warnings; cargo test --locked
#   5. licences:            cargo deny check licenses bans sources (both workspaces, as host/deny.toml says)
#                           python3 scripts/third-party-notices.py --check
# Not covered: the real-app end-to-end test and the deb/AppImage bundles (CI after the flip).
#
# One-time setup (Ubuntu 24.04):
#   - root: apt-get install -y build-essential pkg-config git python3-jsonschema \
#       libxkbcommon-dev libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
#     (the -dev packages are the ones .github/workflows/host.yml and desktop-app.yml install).
#     In WSL, where sudo may want a password: wsl.exe -d Ubuntu -u root -- apt-get ...
#   - rustup (https://rustup.rs, --default-toolchain none): host/rust-toolchain.toml picks the
#     toolchain. Node 22 or newer (host/crates/sdcompat's tests; without it they pass without
#     running, which the summary lists). cargo-deny is installed on the first run.
#
# Environment knobs:
#   REPO=<this checkout>    checkout to export from
#   REF=HEAD                commit or branch to test
#   PD=$HOME/pd             work folder (src/, target/, logs/), on Linux's own disk
#   SKIP_APP=1              skip step 4
#   SKIP_DENY=1             skip cargo deny (and its first-time install)
#   KEEP_DISPLAY=1          keep DISPLAY/WAYLAND_DISPLAY (default: unset, like CI's headless runner;
#                           with them, tests that start a host may ask the desktop for input access)
#
# Exit status: 0 if every step passed, 1 otherwise. Logs: $PD/logs/<stamp>/, the summary also
# in $PD/logs/latest-summary.txt.
set -uo pipefail

REPO=${REPO:-$(cd "$(dirname "$0")/.." && pwd)}
REF=${REF:-HEAD}
PD=${PD:-$HOME/pd}
SRC=$PD/src
STAMP=$(date +%Y%m%d-%H%M%S)
LOGS=$PD/logs/$STAMP
export CARGO_TARGET_DIR=$PD/target
export CARGO_TERM_COLOR=never
export NO_COLOR=1

# Linux tools only. WSL appends the Windows PATH, which can carry cargo.exe, Windows npm and
# other Windows builds of POSIX tools; none of them may resolve here.
PATH="$(printf '%s' "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd:)"
PATH="$HOME/.cargo/bin:$PATH"
[ -d "$HOME/.local/node22/bin" ] && PATH="$HOME/.local/node22/bin:$PATH"
export PATH

if [ "${KEEP_DISPLAY:-0}" != 1 ]; then
  unset DISPLAY WAYLAND_DISPLAY XDG_SESSION_TYPE
fi

mkdir -p "$LOGS"
SUMMARY=$LOGS/summary.txt
: > "$SUMMARY"
say() { printf '%s\n' "$*" | tee -a "$SUMMARY"; }

declare -a STEP_NAMES STEP_RC STEP_SECS
FAILED=0

# step <name> <dir> <command...>: runs it, logs to $LOGS/<name>.log, records rc and time.
step() {
  local name=$1 dir=$2; shift 2
  local log=$LOGS/$name.log t0=$SECONDS rc
  printf '>> %-28s ' "$name"
  ( cd "$dir" && "$@" ) > "$log" 2>&1
  rc=$?
  local dt=$((SECONDS - t0))
  STEP_NAMES+=("$name"); STEP_RC+=("$rc"); STEP_SECS+=("$dt")
  if [ "$rc" -eq 0 ]; then echo "ok    (${dt}s)"; else echo "FAIL  rc=$rc (${dt}s)  log: $log"; FAILED=1; fi
  return "$rc"
}

skip_step() {
  STEP_NAMES+=("$1"); STEP_RC+=("skip"); STEP_SECS+=(0)
  printf '>> %-28s skipped (%s)\n' "$1" "$2"
}

# Per-crate totals from a cargo test log. Integration-test binaries are named after their
# file (tests/e2e.rs -> e2e), so they are credited to the package whose unittests ran last
# (cargo runs each package's binaries together); doc-tests name their crate.
test_totals() {
  awk '
    /^ *Running unittests / {
      b = $0; sub(/.*\//, "", b); sub(/\)$/, "", b); sub(/-[0-9a-f]+$/, "", b)
      pkg = b; cur = pkg " (unit " $3 ")"; next }
    /^ *Running / {
      b = $0; sub(/.*\//, "", b); sub(/\)$/, "", b); sub(/-[0-9a-f]+$/, "", b)
      cur = pkg " (tests/" b ")"; next }
    /^ *Doc-tests / { pkg = $2; cur = $2 " (doc)"; next }
    /^test result:/ {
      p = f = i = 0
      for (k = 1; k <= NF; k++) {
        if ($(k+1) ~ /^passed/)  p = $k
        if ($(k+1) ~ /^failed/)  f = $k
        if ($(k+1) ~ /^ignored/) i = $k
      }
      P[pkg] += p; F[pkg] += f; I[pkg] += i; tp += p; tf += f; ti += i
      if (!(pkg in seen)) { seen[pkg] = 1; order[++n] = pkg }
      if (f > 0) bad[++nb] = cur ": " f " failed"
    }
    END {
      for (k = 1; k <= n; k++) printf "  %-26s %5d passed %3d failed %3d ignored\n", order[k], P[order[k]], F[order[k]], I[order[k]]
      printf "  %-26s %5d passed %3d failed %3d ignored\n", "TOTAL", tp, tf, ti
      for (k = 1; k <= nb; k++) print "  failing binary: " bad[k]
    }' "$1"
}

# Tests that pass by returning early print "skipped: ..." (sdcompat, no Node 22) or
# "skipping: ..." (plugins, no Python); --show-output puts them under "---- <test> stdout ----".
test_skips() {
  awk '/^---- .* stdout ----$/ { t = $2; next }
       /^(successes|failures):$/ { t = ""; next }
       t != "" && /(^|[^_[:alnum:]])[Ss]kipp(ed|ing):/ { print "  " t ": " $0 }' "$1" | sort -u
}

failure_digest() {
  grep -nE 'panicked at|^test .* \.\.\. FAILED|^error(\[|:)|^warning: |^Diff in |^failures:$' "$1" | head -40 | sed 's/^/  /'
}

T0=$SECONDS

# ---- 1. Export the committed tree --------------------------------------------------------
GIT="git -c safe.directory=$REPO -C $REPO"
COMMIT=$($GIT rev-parse "$REF") || { echo "cannot resolve $REF in $REPO"; exit 1; }
rm -rf "$SRC"
mkdir -p "$SRC"
# -m: files get the extraction time, so cargo never mistakes a new tree for an old build.
$GIT archive "$COMMIT" | tar -xm -C "$SRC" || { echo "git archive failed"; exit 1; }

# Toolchain from host/rust-toolchain.toml (no-op once installed).
( cd "$SRC/host" && { rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install; } ) > "$LOGS/toolchain.log" 2>&1

say "PerritoDeck Linux check  $STAMP"
say "commit   $COMMIT  ($($GIT log -1 --format='%cd %s' --date=short "$COMMIT"))"
say "os       $(. /etc/os-release; echo "$PRETTY_NAME"), kernel $(uname -r), $(nproc) cpus"
say "rustc    $(cd "$SRC/host" && rustc -V 2>/dev/null)"
say "cargo    $(cd "$SRC/host" && cargo -V 2>/dev/null)"
say "node     $(node -v 2>/dev/null || echo missing), npm $(npm -v 2>/dev/null || echo missing)"
say "python   $(python3 -V 2>&1), jsonschema $(python3 -c 'import importlib.metadata as m;print(m.version("jsonschema"))' 2>/dev/null || echo missing)"
say "display  DISPLAY=${DISPLAY:-<unset>} WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-<unset>}"
say "target   $CARGO_TARGET_DIR"
say "logs     $LOGS"
say ""

# ---- 2. host/ workspace ------------------------------------------------------------------
step host-fmt     "$SRC/host" cargo fmt --all --check
step host-clippy  "$SRC/host" cargo clippy --all-targets --locked -- -D warnings
step host-test    "$SRC/host" cargo test --locked --no-fail-fast -- --show-output

# ---- 3. protocol --------------------------------------------------------------------------
step protocol     "$SRC"      python3 protocol/tools/validate.py protocol

# ---- 4. desktop app (its own Cargo workspace) --------------------------------------------
if [ "${SKIP_APP:-0}" = 1 ]; then
  skip_step app-npm-ci "SKIP_APP=1"
elif step app-npm-ci "$SRC/host/app" npm ci --no-audit --no-fund \
  && step app-npm-build "$SRC/host/app" npm run build; then
  step app-fmt    "$SRC/host/app/src-tauri" cargo fmt --check
  step app-clippy "$SRC/host/app/src-tauri" cargo clippy --locked --all-targets -- -D warnings
  step app-test   "$SRC/host/app/src-tauri" cargo test --locked --no-fail-fast -- --show-output
else
  skip_step app-clippy "npm step failed"
fi

# ---- 5. licences --------------------------------------------------------------------------
if [ "${SKIP_DENY:-0}" = 1 ]; then
  skip_step deny "SKIP_DENY=1"
else
  if ! command -v cargo-deny >/dev/null 2>&1; then
    # Built with the pinned toolchain (run from host/), in a throwaway target dir.
    step deny-install "$SRC/host" env -u CARGO_TARGET_DIR cargo install --locked cargo-deny
  fi
  if command -v cargo-deny >/dev/null 2>&1; then
    say "cargo-deny $(cargo-deny --version 2>/dev/null)"
    step deny-host "$SRC/host" cargo deny check licenses bans sources
    step deny-app  "$SRC/host" cargo deny --manifest-path app/src-tauri/Cargo.toml --config deny.toml check licenses bans sources
  else
    skip_step deny-host "cargo-deny not installed"
  fi
fi
step notices "$SRC/host" python3 ../scripts/third-party-notices.py --check

TOTAL=$((SECONDS - T0))

# ---- Summary -------------------------------------------------------------------------------
say ""
say "Results"
for k in "${!STEP_NAMES[@]}"; do
  rc=${STEP_RC[$k]}
  if [ "$rc" = skip ]; then r="skipped"; elif [ "$rc" -eq 0 ]; then r="ok"; else r="FAIL (rc $rc)"; fi
  say "$(printf '  %-16s %-14s %5ss' "${STEP_NAMES[$k]}" "$r" "${STEP_SECS[$k]}")"
done
say "  total time       $((TOTAL / 60))m $((TOTAL % 60))s"

for t in host-test app-test; do
  [ -f "$LOGS/$t.log" ] || continue
  say ""
  say "Test totals: $t"
  test_totals "$LOGS/$t.log" | tee -a "$SUMMARY"
  sk=$(test_skips "$LOGS/$t.log")
  if [ -n "$sk" ]; then say "Tests that returned early (passed without running):"; say "$sk"; fi
done

for k in "${!STEP_NAMES[@]}"; do
  rc=${STEP_RC[$k]}
  [ "$rc" = skip ] || [ "$rc" -eq 0 ] && continue
  say ""
  say "Failure digest: ${STEP_NAMES[$k]}  ($LOGS/${STEP_NAMES[$k]}.log)"
  failure_digest "$LOGS/${STEP_NAMES[$k]}.log" | tee -a "$SUMMARY"
done

cp "$SUMMARY" "$PD/logs/latest-summary.txt"
exit "$FAILED"
