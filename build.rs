// Finds LLVM, checks it is the one version Lyftar supports (D2, D13), links it, compiles the C++ shim (D16), and on
// anything but macOS generates the bindings (D14).

use std::path::{Path, PathBuf};
use std::process::Command;

const LLVM_MAJOR: &str = "23";

#[cfg(not(target_os = "macos"))]
include!("tools/bindings/generate.rs");

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-changed=tools/bindings/generate.rs");
    println!("cargo:rerun-if-env-changed=LYFTAR_LLVM_CONFIG");
    println!("cargo:rerun-if-env-changed=LYFTAR_SANITIZE");

    let llvm_config = find_llvm_config()?;
    // trace: D13
    let version = run(&llvm_config, &["--version"])?;
    if version.split('.').next() != Some(LLVM_MAJOR) {
        return Err(format!("Lyftar supports LLVM {LLVM_MAJOR} only, and {} is LLVM {version}", llvm_config.display()));
    }

    let library_directory = run(&llvm_config, &["--libdir"])?;
    println!("cargo:rustc-link-search=native={library_directory}");
    for library in run(&llvm_config, &["--link-shared", "--libs"])?.split_whitespace() {
        let name =
            library.strip_prefix("-l").ok_or_else(|| format!("unexpected library from llvm-config: {library}"))?;
        println!("cargo:rustc-link-lib=dylib={name}");
    }
    // LLVM keeps the C API of `lto.h` in a library of its own.
    println!("cargo:rustc-link-lib=dylib=LTO");

    compile_shim(&llvm_config)?;

    let target = std::env::var("CARGO_CFG_TARGET_OS").map_err(|error| format!("no target from Cargo: {error}"))?;
    if target != "macos" {
        generate(Path::new(&run(&llvm_config, &["--includedir"])?))?;
    }
    Ok(())
}

// trace: D25
fn compile_shim(llvm_config: &Path) -> Result<(), String> {
    let mut shim = cc::Build::new();
    shim.cpp(true)
        .file("shim/shim.cpp")
        .file("shim/passes.cpp")
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(true);
    // A plugin built with another compiler against Homebrew's libc++ LLVM crashed `opt` (D25), so on macOS the shim is
    // always built with the Clang beside the LLVM it links.
    if cfg!(target_os = "macos") {
        shim.compiler(Path::new(&run(llvm_config, &["--bindir"])?).join("clang++"));
    }
    // llvm-config says -std=c++17, and the shim is C++23 with libc++ hardening on instead.
    for flag in run(llvm_config, &["--cxxflags"])?.split_whitespace() {
        if let Some(directory) = flag.strip_prefix("-I") {
            // LLVM's own headers do not build under the shim's warnings.
            shim.flag("-isystem").flag(directory);
        } else if !flag.starts_with("-std=") {
            shim.flag(flag);
        }
    }
    shim.flag("-std=c++23");
    shim.define("_LIBCPP_HARDENING_MODE", "_LIBCPP_HARDENING_MODE_FAST");
    // LLVM's inline code keeps its assertions in a debug build of the shim, which catches a broken contract early.
    if std::env::var("PROFILE").is_ok_and(|profile| profile == "release") {
        shim.define("NDEBUG", None);
    }
    // trace: D27
    if std::env::var_os("LYFTAR_SANITIZE").is_some() {
        shim.flag("-fsanitize=address,undefined").flag("-fno-sanitize-recover=all").flag("-fno-omit-frame-pointer");
    }
    shim.try_compile("lyftar_shim").map_err(|error| format!("could not compile the shim: {error}"))
}

#[cfg(not(target_os = "macos"))]
fn generate(include_directory: &Path) -> Result<(), String> {
    let out_directory = std::env::var("OUT_DIR").map_err(|error| format!("no OUT_DIR from Cargo: {error}"))?;
    generate_bindings(include_directory, &PathBuf::from(out_directory).join("bindings.rs"))
}

#[cfg(target_os = "macos")]
fn generate(_include_directory: &Path) -> Result<(), String> {
    Err("building Lyftar on macOS for another platform is not supported: bindgen is not available here (D14)".into())
}

fn find_llvm_config() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("LYFTAR_LLVM_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let candidates = [
        format!("llvm-config-{LLVM_MAJOR}"),
        format!("/opt/homebrew/opt/llvm@{LLVM_MAJOR}/bin/llvm-config"),
        "/opt/homebrew/opt/llvm/bin/llvm-config".to_string(),
        "llvm-config".to_string(),
    ];
    for candidate in &candidates {
        let found = Command::new(candidate).arg("--version").output();
        if found.is_ok_and(|output| String::from_utf8_lossy(&output.stdout).split('.').next() == Some(LLVM_MAJOR)) {
            return Ok(PathBuf::from(candidate));
        }
    }
    Err(format!("could not find llvm-config for LLVM {LLVM_MAJOR}; set LYFTAR_LLVM_CONFIG to its path"))
}

fn run(program: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run {}: {error}", program.display()))?;
    if !output.status.success() {
        return Err(format!("{} {arguments:?} failed: {}", program.display(), String::from_utf8_lossy(&output.stderr)));
    }
    let printed = String::from_utf8(output.stdout)
        .map_err(|error| format!("{} printed something that is not UTF-8: {error}", program.display()))?;
    Ok(printed.trim().to_string())
}
