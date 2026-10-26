#!/usr/bin/env bash
# Tauri's universal-apple-darwin build merges only the app's main binary with lipo, but its
# bundler then copies every binary the crate has from target/universal-apple-darwin/release,
# and stops when one is missing. This merges the others (src/bin/*.rs: perritodeck-mcp, the
# bridge for AI assistants) the same way. It runs as Tauri's beforeBundleCommand, after both
# architectures are built and before bundling: desktop-app.yml and release.yml pass it with
# --config for a universal build. Building a universal app on a Mac by hand needs it too:
#   npx tauri build --target universal-apple-darwin \
#     --config '{"build":{"beforeBundleCommand":"bash ../../scripts/lipo-universal-bins.sh"}}'
set -euo pipefail
app="$(cd "$(dirname "$0")/../host/app/src-tauri" && pwd)"
target="${CARGO_TARGET_DIR:-$app/target}"
out="$target/universal-apple-darwin/release"
mkdir -p "$out"
for src in "$app"/src/bin/*.rs; do
  bin="$(basename "$src" .rs)"
  lipo -create "$target/aarch64-apple-darwin/release/$bin" "$target/x86_64-apple-darwin/release/$bin" \
    -output "$out/$bin"
  echo "$bin: $(lipo -archs "$out/$bin")"
done
