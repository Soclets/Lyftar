//! What a plugin does with Lyftar's static LLVM (D53).

#![cfg(all(lyftar_static_llvm, not(target_os = "windows")))]

#[test]
fn decision_53_a_plugin_does_not_compile_with_the_static_llvm() {
    trybuild::TestCases::new().compile_fail("tests/static_compile_fail/*.rs");
}
