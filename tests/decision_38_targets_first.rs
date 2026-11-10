//! A target is found by triple or name before anything else in the process has run, which is why this file has one test (D38).

#[test]
fn decision_38_a_target_is_found_before_anything_else_has_run() {
    let triple = lyftar::target::default_target_triple();
    assert!(lyftar::target::Target::from_triple(&triple).is_ok(), "{triple}");
    assert!(
        lyftar::target::Target::from_name("aarch64").is_some() || lyftar::target::Target::from_name("x86-64").is_some()
    );
}
