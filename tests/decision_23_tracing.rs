//! The tracing check from `IMPLEMENTATION.md`, which D23 brought in: it fails on a tag that names no decision, and lists the
//! decisions nothing traces yet.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Mutex;

fn run_check(repository: &Path) -> Output {
    // Both tests build the check into the same folder, and one could replace it while the other is starting it.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = env!("CARGO_MANIFEST_DIR");
    Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--manifest-path"])
        .arg(format!("{root}/tools/tracing/Cargo.toml"))
        .arg("--target-dir")
        .arg(format!("{root}/target/tools"))
        .arg("--")
        .arg(repository)
        .output()
        .expect("running the tracing check")
}

#[test]
fn decision_23_every_tag_names_a_decision_and_only_the_process_decisions_go_untraced() {
    let output = run_check(Path::new(env!("CARGO_MANIFEST_DIR")));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}{}", String::from_utf8_lossy(&output.stderr));
    // D18, D20, D28 and D29 settle how sessions work, or were replaced, so no code carries them out. D37 was replaced by D43.
    assert!(text.contains("nothing traces yet: D18, D20, D28, D29, D37\n"), "{text}");
}

#[test]
fn decision_23_a_tag_naming_no_decision_fails_the_check() {
    let directory = std::env::temp_dir().join(format!("lyftar-tracing-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    std::fs::write(directory.join("DECISIONS.md"), "**D1. One.** Text.\n**D2. Two.** Text.\n").expect("a file");
    std::fs::write(directory.join("good.rs"), concat!("// trace", ": D1\n")).expect("a file");
    let passing = run_check(&directory);
    assert!(
        passing.status.success(),
        "{}{}",
        String::from_utf8_lossy(&passing.stdout),
        String::from_utf8_lossy(&passing.stderr)
    );
    assert!(String::from_utf8_lossy(&passing.stdout).contains("nothing traces yet: D2\n"));

    // Spelled apart, so that this file doesn't fail the check on this repository.
    let bad = concat!("// trace", ": D999\n// backtrace", ": D5\n");
    std::fs::write(directory.join("bad.rs"), bad).expect("a file");
    let failing = run_check(&directory);
    assert!(!failing.status.success());
    let text = String::from_utf8_lossy(&failing.stdout);
    assert!(text.contains("D999 names no decision"), "{text}");
    assert!(!text.contains("D5"), "{text}");
    let _ = std::fs::remove_dir_all(directory);
}
