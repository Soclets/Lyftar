//! What a plugin does on Windows (D44).

#![cfg(target_os = "windows")]

#[test]
fn decision_44_a_plugin_does_not_compile_on_windows() {
    trybuild::TestCases::new().compile_fail("tests/windows_compile_fail/*.rs");
}
