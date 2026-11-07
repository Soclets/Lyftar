//! Verus proves Lyftar's own checks (D21, D22). Verus is a tool for working on Lyftar, not for using it: nothing here runs
//! when a crate depends on Lyftar.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn verus() -> PathBuf {
    if let Some(path) = std::env::var_os("LYFTAR_VERUS") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").expect("HOME is set");
    let path = Path::new(&home).join(".local/verus/verus-arm64-macos/verus");
    assert!(path.exists(), "Verus isn't at {} (LYFTAR_VERUS can say where it is)", path.display());
    path
}

/// Each file of proven checks: Lyftar's, and the stack map reader's, which has to stay free of Lyftar (D46).
const CHECKS: [&str; 2] = ["src/checks.rs", "stack-maps/src/checks.rs"];

fn source(file: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).expect("reading the checks")
}

#[test]
fn decision_22_verus_proves_the_checks() {
    for file in CHECKS {
        let output = Command::new(verus())
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([file, "--crate-type=lib", "--", "-Zcrate-attr=feature(proc_macro_hygiene)"])
            .output()
            .expect("running Verus");
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{file}: {text}");
        let result = text.lines().find(|line| line.starts_with("verification results::")).expect("Verus's result line");
        assert!(result.ends_with(" 0 errors"), "{file}: {result}");
        let verified: u32 = result
            .trim_start_matches("verification results:: ")
            .split_whitespace()
            .next()
            .and_then(|count| count.parse().ok())
            .expect("a count");
        let functions = u32::try_from(source(file).matches("pub(crate) fn ").count()).expect("a small count");
        assert!(verified >= functions, "{file}: {verified} verified, for {functions} checks: {result}");
    }
}

#[test]
fn decision_22_every_check_has_a_specification() {
    for file in CHECKS {
        let source = source(file);
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if line.starts_with("pub(crate) fn ") {
                let attributes = lines[..index].iter().rev().take_while(|above| !above.starts_with("///"));
                assert!(
                    attributes.clone().any(|above| above.contains("verus_spec")),
                    "this check isn't proven: {line}"
                );
            }
        }
    }
}

#[test]
fn decision_22_the_assumptions_are_written_down_in_one_place() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ASSUMPTIONS.md");
    let text = std::fs::read_to_string(path).expect("ASSUMPTIONS.md");
    // Each entry the checks and the shim point at.
    for entry in ["uniques types", "sort_unstable", "generation", "WeakVH", "moves that metadata"] {
        assert!(text.contains(entry), "ASSUMPTIONS.md says nothing about {entry}");
    }
}
