//! `coverage.json` is true: every raw function is wrapped, or not with a reason, and every inkwell API the projects use has a
//! Lyftar equivalent that exists.

use std::fs;

use serde_json::Value;

fn manifest_file(path: &str) -> String {
    fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).expect("a file of this repository")
}

/// All the crate's source, with comments gone, since a comment saying why something isn't called doesn't make it wrapped.
fn source_without_comments() -> String {
    let directory = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut source = String::new();
    for entry in fs::read_dir(directory).expect("the source") {
        let path = entry.expect("the source").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            for line in fs::read_to_string(&path).expect("the source").lines() {
                source.push_str(line.split("//").next().unwrap_or_default());
                source.push('\n');
            }
        }
    }
    source
}

fn mentions_name(source: &str, name: &str) -> bool {
    source.match_indices(name).any(|(start, _)| {
        let is_name = |character: char| character.is_alphanumeric() || character == '_';
        source[..start].chars().next_back().is_none_or(|character| !is_name(character))
            && source[start + name.len()..].chars().next().is_none_or(|character| !is_name(character))
    })
}

fn coverage() -> Value {
    serde_json::from_str(&manifest_file("coverage.json")).expect("coverage.json is JSON")
}

#[test]
fn decision_16_every_raw_function_is_wrapped_or_not_with_a_reason() {
    let coverage = coverage();
    let listed = coverage["raw"].as_object().expect("a raw section");
    let bindings = manifest_file("src/raw/macos.rs");
    let source = source_without_comments();
    let mut problems = Vec::new();
    let mut count = 0;
    for piece in bindings.split("pub fn ").skip(1) {
        let name: String =
            piece.chars().take_while(|character| character.is_alphanumeric() || *character == '_').collect();
        count += 1;
        let Some(entry) = listed.get(&name) else {
            problems.push(format!("{name} isn't listed"));
            continue;
        };
        let mentioned = mentions_name(&source, &name);
        match entry["status"].as_str() {
            Some("wrapped") if mentioned => {}
            Some("wrapped") => problems.push(format!("{name} says wrapped, and nothing calls it")),
            Some("not_wrapped") => {
                if mentioned {
                    problems.push(format!("{name} says not wrapped, and something calls it"));
                }
                if entry["reason"].as_str().is_none_or(str::is_empty) {
                    problems.push(format!("{name} has no reason"));
                }
            }
            _ => problems.push(format!("{name} is unclassified")),
        }
    }
    assert_eq!(count, listed.len(), "coverage.json lists functions the bindings don't have");
    assert!(problems.is_empty(), "run tools/coverage, then see:\n{}", problems.join("\n"));
}

#[test]
fn decision_16_every_inkwell_api_the_projects_use_has_an_equivalent() {
    let coverage = coverage();
    let source = source_without_comments();
    let mut problems = Vec::new();
    for section in ["methods", "types"] {
        for (name, entry) in coverage["inkwell"][section].as_object().expect("an inkwell section") {
            let Some(lyftar) = entry["lyftar"].as_str() else {
                problems.push(format!("{name} has no equivalent"));
                continue;
            };
            // Each `Owner::name` an entry names has to exist in the source: a macro can be what makes it a function.
            for token in lyftar.split([',', ';', ' ']) {
                let Some((_, name_part)) = token.rsplit_once("::") else { continue };
                let last: String = name_part
                    .chars()
                    .take_while(|character| character.is_alphanumeric() || *character == '_')
                    .collect();
                if last.is_empty() || !last.chars().next().is_some_and(char::is_lowercase) {
                    continue;
                }
                if !mentions_name(&source, &last) {
                    problems.push(format!("{name}: {token} doesn't exist"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
