//! Linking with lld inside Lyftar (D49, D52). With an LLVM that has no lld, such as Homebrew's, `lld` says so instead.

use lyftar::linking::LldFlavor;
use lyftar::lld;

#[cfg(lyftar_has_lld)]
#[test]
fn decision_52_lld_runs_again_after_a_link_that_fails() {
    let error = lld(LldFlavor::Gnu, &["no-such-object.o", "-o", "never"]).expect_err("nothing to link");
    assert!(error.message().contains("no-such-object.o"), "{}", error.message());
    let version = lld(LldFlavor::Gnu, &["--version"]).expect("lld's version");
    assert!(version.contains("LLD 23"), "{version}");
    let version = lld(LldFlavor::Darwin, &["--version"]).expect("lld's version");
    assert!(version.contains("LLD 23"), "{version}");
}

#[cfg(not(lyftar_has_lld))]
#[test]
fn decision_52_lld_says_when_this_lyftar_has_none() {
    let error = lld(LldFlavor::Gnu, &["--version"]).expect_err("no lld");
    assert!(error.message().contains("no lld libraries"), "{}", error.message());
}

#[test]
#[should_panic(expected = "lld: \"with\\0nul\" has a NUL byte inside it")]
fn decision_3_an_lld_argument_with_a_nul_panics() {
    let _ = lld(LldFlavor::Gnu, &["with\0nul"]);
}
