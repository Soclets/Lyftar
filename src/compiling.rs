//! Compiling C to an object with clang inside Lyftar, so a compiler built on it can build C libraries, such as CORE-MATH, with
//! nothing installed (D55).
//!
//! Only C, only to an object: [`link`](crate::link) links it. clang's own headers, the ones C on ARM and x86 can reach, are built
//! in (D60). Libc's headers are the caller's, through [`CCompileOptions::include_folders`]: no system folder is searched. Compiles
//! can run at once, each with a compiler of its own.
//!
//! A compiler that never calls [`compile_c`] still carries about 8.6 MB of clang, which the README measures.

// trace: D61
// No Cargo feature leaves clang out: Lyftar stays one crate with nothing to configure.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use crate::bad_call::bad_call;
use crate::error::Error;
use crate::linking::{OLDEST_MAC_OS, SystemTarget};
use crate::passes::OptimizationLevel;
use crate::text::c_string;

/// What [`compile_c`] needs besides the file: how hard to optimize, where headers are, and which macros to define.
#[derive(Clone, Copy, Debug)]
pub struct CCompileOptions<'a> {
    // trace: D57
    /// How hard to optimize, `-O0` to `-O3`. `-O2` by default.
    pub optimization: OptimizationLevel,
    /// Searched for `#include`, in order, before clang's own headers, as `-I` is.
    pub include_folders: &'a [&'a Path],
    // trace: D58
    /// Macros to define before the file: `(name, None)` as `-Dname`, which defines it as 1, and `(name, Some(value))` as
    /// `-Dname=value`.
    pub defines: &'a [(&'a str, Option<&'a str>)],
}

impl Default for CCompileOptions<'_> {
    fn default() -> Self {
        CCompileOptions { optimization: OptimizationLevel::Level2, include_folders: &[], defines: &[] }
    }
}

impl SystemTarget {
    /// What clang's driver is told about the target: its triple, and for the Cortex-M4 its processor and hard-float unit.
    fn clang_target(self) -> Vec<String> {
        match self {
            SystemTarget::MacOsAarch64 => vec![format!("--target=arm64-apple-macosx{OLDEST_MAC_OS}")],
            SystemTarget::LinuxAarch64 => vec!["--target=aarch64-unknown-linux-musl".to_string()],
            SystemTarget::LinuxX86_64 => vec!["--target=x86_64-unknown-linux-musl".to_string()],
            SystemTarget::CortexM4 => {
                ["--target=thumbv7em-none-eabihf", "-mcpu=cortex-m4", "-mfloat-abi=hard", "-mfpu=fpv4-sp-d16"]
                    .map(String::from)
                    .to_vec()
            }
        }
    }
}

// trace: D55
/// Compiles one C file to an object for `target`, with clang inside Lyftar. Gives back clang's warnings, which may be empty.
///
/// # Errors
/// clang's errors, if the file doesn't compile, or why it can't compile at all, such as an LLVM without clang.
///
/// # Panics
/// If a path isn't valid UTF-8 or has a NUL byte inside it, or a define's name is empty or has `=` or a NUL byte in it, or its
/// value has a NUL byte (D3).
#[track_caller]
pub fn compile_c(
    target: SystemTarget,
    source: &Path,
    output: &Path,
    options: &CCompileOptions<'_>,
) -> Result<String, Error> {
    let text = |path: &Path| -> String {
        match path.to_str() {
            Some(text) => text.to_string(),
            None => bad_call(format_args!(
                "compile_c: {} isn't valid UTF-8, which clang's arguments have to be",
                path.display()
            )),
        }
    };
    let source = text(source);
    let output = text(output);
    let mut arguments = target.clang_target();
    let level = match options.optimization {
        OptimizationLevel::Level0 => "-O0",
        OptimizationLevel::Level1 => "-O1",
        OptimizationLevel::Level2 => "-O2",
        OptimizationLevel::Level3 => "-O3",
    };
    arguments.push(level.to_string());
    for folder in options.include_folders {
        arguments.extend(["-I".to_string(), text(folder)]);
    }
    for (name, value) in options.defines {
        if name.is_empty() || name.contains('=') {
            bad_call(format_args!("compile_c: {name:?} isn't a macro's name"));
        }
        arguments.push(match value {
            Some(value) => format!("-D{name}={value}"),
            None => format!("-D{name}"),
        });
    }
    // Only clang's headers and the caller's folders, and the file is C whatever its name.
    arguments.extend(["-nostdlibinc".to_string(), "-x".to_string(), "c".to_string(), source]);
    // Every argument is checked before anything can fail at run time, so a bad call panics with or without clang.
    let mut arguments: Vec<CString> = arguments.iter().map(|argument| c_string(argument, "compile_c")).collect();
    let output = c_string(&output, "compile_c");
    let resources = clang_resources()?;
    arguments.extend(["-resource-dir", &text(&resources)].map(|argument| c_string(argument, "compile_c")));
    run_clang(&arguments, &output)
}

#[cfg(lyftar_has_clang)]
fn run_clang(arguments: &[CString], output: &CString) -> Result<String, Error> {
    crate::target::set_up_targets();
    let pointers: Vec<*const std::ffi::c_char> = arguments.iter().map(|argument| argument.as_ptr()).collect();
    let mut messages = std::ptr::null_mut();
    // SAFETY: the arguments and the output are C strings that outlive the call, every target is set up, and clang writes the
    // messages, which this side frees.
    let written =
        unsafe { crate::shim::lyftar_compile_c(pointers.as_ptr(), pointers.len(), output.as_ptr(), &raw mut messages) };
    let printed = Error::from_owned_message(messages).message().to_string();
    if written { Ok(printed) } else { Err(Error::new(printed)) }
}

#[cfg(not(lyftar_has_clang))]
fn run_clang(_arguments: &[CString], _output: &CString) -> Result<String, Error> {
    Err(Error::new(
        "this Lyftar was built with an LLVM that has no clang libraries; Lyftar's own static LLVM has them (D55)",
    ))
}

/// The folder clang's built-in headers are written to, once per process, as clang's resource folder: they go in its `include`.
#[cfg(lyftar_has_clang)]
fn clang_resources() -> Result<PathBuf, Error> {
    mod built_in {
        include!(concat!(env!("OUT_DIR"), "/clang_headers.rs"));
    }
    static WRITTEN: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    let written = WRITTEN.get_or_init(|| {
        let folder = std::env::temp_dir().join(format!("lyftar-clang-{}", std::process::id()));
        let include = folder.join("include");
        std::fs::create_dir_all(&include).map_err(|error| format!("{}: {error}", include.display()))?;
        for (name, bytes) in built_in::FILES {
            let path = include.join(name);
            std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(folder)
    });
    written.clone().map_err(Error::new)
}

#[cfg(not(lyftar_has_clang))]
fn clang_resources() -> Result<PathBuf, Error> {
    Err(Error::new(
        "this Lyftar was built with an LLVM that has no clang libraries; Lyftar's own static LLVM has them (D55)",
    ))
}
