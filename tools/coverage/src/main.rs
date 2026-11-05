// Brings `coverage.json` up to date with the source, keeping what it says about what isn't wrapped:
//   cargo run --manifest-path tools/coverage/Cargo.toml
//
// A raw function a safe wrapper calls is "wrapped". One nothing calls keeps its reason, or goes in as "unclassified", which
// `tests/coverage.rs` fails on, until someone writes why it isn't wrapped.

use std::collections::BTreeMap;
use std::fs;

use serde_json::{Map, Value, json};

fn main() -> Result<(), String> {
    let path = "coverage.json";
    let read = |file: &str| fs::read_to_string(file).map_err(|error| format!("{file}: {error}"));
    let raw = read("src/raw/macos.rs")?;
    let mut source = String::new();
    let entries = fs::read_dir("src").map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?.path();
        if entry.extension().is_some_and(|extension| extension == "rs") {
            // A comment saying why something isn't called doesn't make it wrapped.
            for line in read(&entry.display().to_string())?.lines() {
                source.push_str(line.split("//").next().unwrap_or_default());
                source.push('\n');
            }
        }
    }
    let existing: Value = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?,
        Err(_) => json!({}),
    };
    let old_raw = existing.get("raw").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut functions = BTreeMap::new();
    for piece in raw.split("pub fn ").skip(1) {
        let name: String = piece.chars().take_while(|character| character.is_alphanumeric() || *character == '_').collect();
        functions.insert(name, ());
    }
    let mut new_raw = Map::new();
    for name in functions.keys() {
        let wrapped = is_mentioned(&source, name);
        let entry = if wrapped {
            json!({ "status": "wrapped" })
        } else {
            match old_raw.get(name).and_then(|old| old.get("reason")) {
                Some(reason) => json!({ "status": "not_wrapped", "reason": reason }),
                None => json!({ "status": "unclassified" }),
            }
        };
        new_raw.insert(name.clone(), entry);
    }
    let mut output = Map::new();
    output.insert("raw".to_string(), Value::Object(new_raw));
    if let Some(inkwell) = existing.get("inkwell") {
        output.insert("inkwell".to_string(), inkwell.clone());
    }
    let text = serde_json::to_string_pretty(&Value::Object(output)).map_err(|error| error.to_string())?;
    fs::write(path, text + "\n").map_err(|error| format!("{path}: {error}"))
}

fn is_mentioned(source: &str, name: &str) -> bool {
    source.match_indices(name).any(|(start, _)| {
        let is_name_character = |character: char| character.is_alphanumeric() || character == '_';
        let before = source[..start].chars().next_back().is_none_or(|character| !is_name_character(character));
        let after = source[start + name.len()..].chars().next().is_none_or(|character| !is_name_character(character));
        before && after
    })
}
