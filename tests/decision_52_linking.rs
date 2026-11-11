//! Linking with lld inside Lyftar (D49, D52). With an LLVM that has no lld, such as Homebrew's, `lld` says so instead.

use std::path::Path;

use lyftar::linking::{LldFlavor, SystemTarget};
use lyftar::{link, lld};

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

#[test]
#[should_panic(expected = "link: a linker script is for bare metal, and LinuxAarch64 isn't")]
fn decision_3_a_linker_script_for_a_target_that_is_not_bare_metal_panics() {
    let _ = link(SystemTarget::LinuxAarch64, &[], Path::new("never"), Some(Path::new("script.ld")));
}

#[cfg(unix)]
#[test]
#[should_panic(expected = "isn't valid UTF-8, which lld's arguments have to be")]
fn decision_3_a_path_that_is_not_text_panics() {
    use std::os::unix::ffi::OsStrExt;
    let object = Path::new(std::ffi::OsStr::from_bytes(b"object\xff.o"));
    let _ = link(SystemTarget::LinuxAarch64, &[object], Path::new("never"), None);
}
