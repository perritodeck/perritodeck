//! Every golden fixture in `protocol/fixtures` must decode and re-encode to the same JSON.

use perritodeck_protocol::{decode, Decoded, KNOWN_TYPES};
use serde_json::Value;
use std::{collections::HashSet, fs, path::PathBuf};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../protocol/fixtures")
}

#[test]
fn fixtures_round_trip() {
    let mut seen_types = HashSet::new();
    let mut count = 0;
    for entry in fs::read_dir(fixtures_dir()).expect("protocol/fixtures exists") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let original: Value = serde_json::from_str(&text).unwrap();
        let env = match decode(&text) {
            Ok(Decoded::Message(env)) => env,
            other => panic!(
                "{}: expected a known message, got {other:?}",
                path.display()
            ),
        };
        seen_types.insert(env.msg.type_name());
        let reencoded: Value = serde_json::from_str(&env.encode()).unwrap();
        assert_eq!(original, reencoded, "{} did not round-trip", path.display());
        count += 1;
    }
    assert!(count > 0, "no fixtures found");
    for t in KNOWN_TYPES {
        assert!(
            seen_types.contains(t),
            "no fixture covers message type `{t}`"
        );
    }
}
