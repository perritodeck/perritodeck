"""Run every plugin test, and check the bundled SDK copies match the SDK.

    python3 plugins/test_all.py
"""

import filecmp
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SDK = os.path.join(HERE, "sdk-python", "perritodeck_plugin.py")

failed = False
# Tests of the tools in this folder itself (the catalogue builder).
for test in sorted(f for f in os.listdir(HERE) if f.startswith("test_") and f.endswith(".py") and f != "test_all.py"):
    r = subprocess.run([sys.executable, "-m", "unittest", test], cwd=HERE)
    failed |= r.returncode != 0
for name in sorted(os.listdir(HERE)):
    folder = os.path.join(HERE, name)
    if not os.path.isdir(folder):
        continue
    copy = os.path.join(folder, "perritodeck_plugin.py")
    if name != "sdk-python" and os.path.exists(copy) and not filecmp.cmp(SDK, copy, shallow=False):
        print(f"{name}/perritodeck_plugin.py differs from sdk-python; copy it again")
        failed = True
    for test in sorted(f for f in os.listdir(folder) if f.startswith("test_") and f.endswith(".py")):
        r = subprocess.run([sys.executable, "-m", "unittest", test], cwd=folder)
        failed |= r.returncode != 0
    if os.path.exists(os.path.join(folder, "package.json")):
        r = subprocess.run(["node", "--test"], cwd=folder)
        failed |= r.returncode != 0
sys.exit(1 if failed else 0)
