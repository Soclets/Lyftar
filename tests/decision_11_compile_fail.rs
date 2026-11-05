//! What doesn't compile (D4, D10, D11), each beside a near-identical program that does, so a failure can't be for the wrong
//! reason. The twins in `tests/compile_pass` are also run, against real LLVM.

#[test]
fn decision_11_these_programs_do_not_compile_and_their_twins_do() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/*.rs");
    cases.pass("tests/compile_pass/*.rs");
}
