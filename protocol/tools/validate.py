#!/usr/bin/env python3
"""Validate protocol fixtures against the v1 schema. Usage: python3 protocol/tools/validate.py protocol"""
import json, sys, glob, jsonschema
root = sys.argv[1]
schema = json.load(open(f"{root}/schema/v1.schema.json"))
jsonschema.Draft202012Validator.check_schema(schema)
v = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
bad = 0
for f in sorted(glob.glob(f"{root}/fixtures/*.json")):
    errs = list(v.iter_errors(json.load(open(f))))
    print(("FAIL " if errs else "ok   ") + f.split("/")[-1])
    for e in errs[:3]: print("   ", e.message[:200]); bad += 1
# negative checks
neg = [
  {"v":2,"type":"hello","payload":{}},
  {"v":1,"type":"keyDown","payload":{"keyIndex":-1,"clientTimestampMs":0}},
  {"v":1,"type":"layout","payload":{"cols":0,"rows":1,"keys":[]}},
  {"v":1,"type":"setImage","payload":{"keyIndex":0,"image":"data:image/gif;base64,AAAA"}},
  {"v":1,"type":"keyDown","payload":{"keyIndex":1,"clientTimestampMs":0,"extra":1}},
  {"v":1,"type":"sliderChange","payload":{"keyIndex":0,"value":101}},
  {"v":1,"type":"layout","payload":{"cols":1,"rows":1,"keys":[{"index":0,"title":"","bgColor":"#000000","textColor":"#FFFFFF","control":"dial"}]}},
]
for n in neg:
    if v.is_valid(n): print("NEGATIVE PASSED (bad):", n); bad += 1
print("negatives ok" if bad == 0 else f"{bad} problems")
sys.exit(1 if bad else 0)
