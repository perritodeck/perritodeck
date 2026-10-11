#!/usr/bin/env bash
# Run a Gradle command; on failure, print compiler errors, lint issues and failing tests
# as GitHub annotations so they're readable without downloading logs.
set -uo pipefail
log="$(mktemp)"
./gradlew --no-daemon "$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
if [ "$status" -ne 0 ]; then
  grep -E '^e: |error:|What went wrong|> .*(failed|FAILED)|Lint found' "$log" | head -15 | sed 's/^/::error::/'
  python3 - <<'PY' || true
import glob, xml.etree.ElementTree as ET
n = 0
for f in glob.glob("app/build/test-results/**/*.xml", recursive=True):
    for case in ET.parse(f).getroot().iter("testcase"):
        for fail in list(case.iter("failure")) + list(case.iter("error")):
            msg = (fail.get("message") or fail.text or "").strip().replace("\n", " ")[:300]
            print(f"::error::{case.get('classname')}.{case.get('name')}: {msg}")
            n += 1
            if n >= 10: raise SystemExit
PY
fi
exit $status
