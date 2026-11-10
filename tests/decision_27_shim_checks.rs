//! `clang-format` in LLVM's style and `clang-tidy` with warnings as errors, on the shim (D27).

// The shim's checks use Homebrew's LLVM on this Mac (D25). Other platforms are not settled.
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::Command;

fn llvm_tool(name: &str) -> PathBuf {
    let output = Command::new("/opt/homebrew/opt/llvm/bin/llvm-config").arg("--bindir").output().expect("llvm-config");
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join(name)
}

fn llvm_flags() -> Vec<String> {
    // The headers of the LLVM this build links, which can be Lyftar's static one (D50), while the tools stay Homebrew's.
    let output = Command::new(env!("LYFTAR_LLVM_CONFIG_IN_USE")).arg("--cxxflags").output().expect("llvm-config");
    let mut flags = vec!["-std=c++23".to_string(), "-DNDEBUG".to_string()];
    for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Some(directory) = flag.strip_prefix("-I") {
            flags.push("-isystem".to_string());
            flags.push(directory.to_string());
        } else if !flag.starts_with("-std=") {
            flags.push(flag.to_string());
        }
    }
    flags.push("-D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_FAST".to_string());
    flags
}

const SHIM: [&str; 5] =
    ["shim/shim.cpp", "shim/passes.cpp", "shim/garbage_collection.cpp", "shim/link.cpp", "shim/shim.h"];

#[test]
fn decision_27_the_shim_is_formatted_in_llvms_style() {
    let output = Command::new(llvm_tool("clang-format"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["--dry-run", "--Werror"])
        .args(SHIM)
        .output()
        .expect("clang-format");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn decision_27_clang_tidy_passes_on_the_shim_with_warnings_as_errors() {
    let output = Command::new(llvm_tool("clang-tidy"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["shim/shim.cpp", "shim/passes.cpp", "shim/garbage_collection.cpp", "shim/link.cpp", "--"])
        .args(llvm_flags())
        .output()
        .expect("clang-tidy");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
