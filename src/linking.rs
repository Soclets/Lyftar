//! Linking, with lld inside Lyftar, so a compiler built on it needs no linker installed (D49, D52).
//!
//! Links run one at a time in a process, since two at once crash lld. A link that fails is an error, and lld can run again after
//! one; if it ever says it can't, every later link is an error that says so.

use std::ffi::{CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::bad_call::bad_call;
use crate::error::Error;
use crate::text::c_string;

/// Which of lld's linkers, by the format it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LldFlavor {
    /// ELF, as `ld.lld`: Linux and bare metal.
    Gnu,
    /// Mach-O, as `ld64.lld`: macOS.
    Darwin,
}

impl LldFlavor {
    /// The name lld's own executable has for this flavour, which lld reads its flavour from.
    fn program_name(self) -> &'static str {
        match self {
            LldFlavor::Gnu => "ld.lld",
            LldFlavor::Darwin => "ld64.lld",
        }
    }
}

/// Whether lld said it can't run again, after which no link in this process can.
static LLD: Mutex<bool> = Mutex::new(false);

// trace: D52
/// Runs lld with its own arguments, those of `ld.lld` or `ld64.lld` after the program's name, and gives back what it printed.
/// For what [`link`](crate::linking) doesn't cover.
///
/// # Errors
/// What lld printed, if the link failed, or why it can't run: this Lyftar's LLVM came without lld, or lld can't run again in
/// this process.
///
/// # Panics
/// If an argument has a NUL byte inside it (D3).
#[track_caller]
pub fn lld(flavor: LldFlavor, arguments: &[&str]) -> Result<String, Error> {
    let mut owned: Vec<CString> = Vec::with_capacity(arguments.len() + 1);
    owned.push(c_string(flavor.program_name(), "lld"));
    owned.extend(arguments.iter().map(|argument| c_string(argument, "lld")));
    let pointers: Vec<*const c_char> = owned.iter().map(|argument| argument.as_ptr()).collect();
    // A poisoned lock only means another link panicked in Rust, outside lld, which leaves it as it was.
    let mut cannot_run_again = LLD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *cannot_run_again {
        return Err(Error::new("lld can't run again in this process, since an earlier link left it unable to"));
    }
    run_lld(&pointers, &mut cannot_run_again)
}

#[cfg(lyftar_has_lld)]
fn run_lld(arguments: &[*const c_char], cannot_run_again: &mut bool) -> Result<String, Error> {
    let mut output = std::ptr::null_mut();
    let mut can_run_again = true;
    // SAFETY: the arguments are C strings that outlive the call, the lock is held, and lld writes both outputs.
    let code = unsafe {
        crate::shim::lyftar_lld(arguments.as_ptr(), arguments.len(), &raw mut output, &raw mut can_run_again)
    };
    *cannot_run_again = !can_run_again;
    let printed = Error::from_owned_message(output).message().to_string();
    if code == 0 { Ok(printed) } else { Err(Error::new(printed)) }
}

#[cfg(not(lyftar_has_lld))]
fn run_lld(_arguments: &[*const c_char], _cannot_run_again: &mut bool) -> Result<String, Error> {
    Err(Error::new(
        "this Lyftar was built with an LLVM that has no lld libraries; Lyftar's own static LLVM has them (D50)",
    ))
}

/// What [`link`] links for: the system and processor a program runs on, which says which system files it needs (D51).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// macOS on Apple Silicon. Lyftar writes the `libSystem` stub itself, from the symbols the objects need.
    MacOsAarch64,
    /// Linux on ARM, linked statically with musl and compiler-rt's builtins into one standalone file.
    LinuxAarch64,
    /// Linux on x86-64, linked statically with musl and compiler-rt's builtins into one standalone file.
    LinuxX86_64,
    /// A Cortex-M4 with its floating-point unit, bare metal: compiler-rt's builtins, and the front end's own startup code and
    /// linker script.
    CortexM4,
}

impl LinkTarget {
    /// The folder of the system files it needs.
    fn system_folder(self) -> &'static str {
        match self {
            LinkTarget::MacOsAarch64 => "macos-aarch64",
            LinkTarget::LinuxAarch64 => "linux-aarch64",
            LinkTarget::LinuxX86_64 => "linux-x86_64",
            LinkTarget::CortexM4 => "cortex-m4",
        }
    }
}

/// The oldest macOS a program linked for [`LinkTarget::MacOsAarch64`] runs on: the first on Apple Silicon.
const OLDEST_MAC_OS: &str = "11.0";

// trace: D49, D51, D52
/// Links object files, and archives of them, into a program for `target`, adding the system files it needs, with lld inside
/// Lyftar: nothing else has to be installed. Gives back what lld printed. A linker script is for bare metal, and without one
/// lld lays the image out itself.
///
/// # Errors
/// What lld printed, if the link failed; an object or archive that can't be read; or why it can't link at all, such as an LLVM
/// without lld or system files that aren't built in.
///
/// # Panics
/// If a path isn't valid UTF-8 or has a NUL byte inside it, or a linker script is given for a target that isn't bare metal (D3).
#[track_caller]
pub fn link(
    target: LinkTarget,
    objects: &[&Path],
    output: &Path,
    linker_script: Option<&Path>,
) -> Result<String, Error> {
    if linker_script.is_some() && target != LinkTarget::CortexM4 {
        bad_call(format_args!("link: a linker script is for bare metal, and {target:?} isn't"));
    }
    let text = |path: &Path| -> String {
        match path.to_str() {
            Some(text) => text.to_string(),
            None => {
                bad_call(format_args!("link: {} isn't valid UTF-8, which lld's arguments have to be", path.display()))
            }
        }
    };
    let objects: Vec<String> = objects.iter().map(|object| text(object)).collect();
    let output = text(output);
    let script = linker_script.map(text);
    let system = system_files()?.join(target.system_folder());
    let in_system = |file: &str| text(&system.join(file));
    let mut arguments: Vec<String> = Vec::new();
    let flavor = match target {
        LinkTarget::MacOsAarch64 => {
            // A folder for each link, since two links can write their stubs while one of them runs.
            static LINKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let stub_folder = system.join(format!("link-{}", LINKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            write_system_stub(&stub_folder, &objects)?;
            arguments.extend(
                ["-arch", "arm64", "-platform_version", "macos", OLDEST_MAC_OS, OLDEST_MAC_OS].map(String::from),
            );
            // No SDK is searched: the stub is the only library.
            arguments.extend(["-syslibroot".to_string(), in_system("no-sdk")]);
            arguments.push(format!("-L{}", text(&stub_folder)));
            arguments.push("-lSystem".to_string());
            arguments.extend(objects);
            LldFlavor::Darwin
        }
        LinkTarget::LinuxAarch64 | LinkTarget::LinuxX86_64 => {
            // Not position-independent, since a stack map holds addresses in a read-only section (D45).
            arguments.push("-static".to_string());
            arguments.extend([in_system("crt1.o"), in_system("crti.o")]);
            arguments.extend(objects);
            arguments.extend([in_system("libc.a"), in_system("libclang_rt.builtins.a"), in_system("crtn.o")]);
            LldFlavor::Gnu
        }
        LinkTarget::CortexM4 => {
            if let Some(script) = script {
                arguments.extend(["-T".to_string(), script]);
            }
            arguments.extend(objects);
            arguments.push(in_system("libclang_rt.builtins.a"));
            LldFlavor::Gnu
        }
    };
    arguments.extend(["-o".to_string(), output]);
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    lld(flavor, &borrowed)
}

/// Writes `libSystem.tbd` into this folder: what libSystem has that these objects need, which is every symbol they
/// leave undefined and none of them defines. No file from Apple is needed (D51).
fn write_system_stub(folder: &Path, objects: &[String]) -> Result<(), Error> {
    let mut defined = std::collections::BTreeSet::new();
    let mut undefined = std::collections::BTreeSet::new();
    for object in objects {
        let (object_defined, object_undefined) = symbols_of(Path::new(object))?;
        defined.extend(object_defined);
        undefined.extend(object_undefined);
    }
    let mut stub = String::from(
        "--- !tapi-tbd\ntbd-version: 4\ntargets: [ arm64-macos ]\ninstall-name: '/usr/lib/libSystem.B.dylib'\n\
         current-version: 1351\nexports:\n  - targets: [ arm64-macos ]\n    symbols: [ ",
    );
    // What lld's lazy binding calls, for macOS before 12, which libSystem has.
    undefined.insert("dyld_stub_binder".to_string());
    let needed: Vec<String> =
        undefined.difference(&defined).map(|symbol| format!("'{}'", symbol.replace('\'', "''"))).collect();
    stub.push_str(&needed.join(", "));
    stub.push_str(" ]\n...\n");
    std::fs::create_dir_all(folder).map_err(|error| Error::new(format!("{}: {error}", folder.display())))?;
    let path = folder.join("libSystem.tbd");
    std::fs::write(&path, stub).map_err(|error| Error::new(format!("{}: {error}", path.display())))
}

/// The global symbols an object file or archive defines, and the ones it leaves undefined.
fn symbols_of(path: &Path) -> Result<(Vec<String>, Vec<String>), Error> {
    let bytes = std::fs::read(path).map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
    read_symbols(path, &bytes)
}

#[cfg(lyftar_has_lld)]
fn read_symbols(path: &Path, bytes: &[u8]) -> Result<(Vec<String>, Vec<String>), Error> {
    let (mut defined, mut undefined) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: the shim reads `bytes.len()` bytes, and on success writes both lists for this side to free.
    let read = unsafe {
        crate::shim::lyftar_object_symbols(bytes.as_ptr().cast(), bytes.len(), &raw mut defined, &raw mut undefined)
    };
    if !read {
        return Err(Error::new(format!("{} isn't an object file or an archive of them", path.display())));
    }
    let lines = |list| Error::from_owned_message(list).message().lines().map(str::to_string).collect::<Vec<String>>();
    Ok((lines(defined), lines(undefined)))
}

#[cfg(not(lyftar_has_lld))]
fn read_symbols(_path: &Path, _bytes: &[u8]) -> Result<(Vec<String>, Vec<String>), Error> {
    Err(Error::new(
        "this Lyftar was built with an LLVM that has no lld libraries; Lyftar's own static LLVM has them (D50)",
    ))
}

/// The folder the built-in system files are written to, once per process, so lld can read them.
#[cfg(lyftar_has_system_files)]
fn system_files() -> Result<PathBuf, Error> {
    mod built_in {
        include!(concat!(env!("OUT_DIR"), "/system_files.rs"));
    }
    static WRITTEN: std::sync::OnceLock<Result<PathBuf, String>> = std::sync::OnceLock::new();
    let written = WRITTEN.get_or_init(|| {
        let folder = std::env::temp_dir().join(format!("lyftar-system-files-{}", std::process::id()));
        for (name, bytes) in built_in::FILES {
            let path = folder.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
            }
            std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(folder)
    });
    written.clone().map_err(Error::new)
}

#[cfg(not(lyftar_has_system_files))]
fn system_files() -> Result<PathBuf, Error> {
    Err(Error::new(
        "this Lyftar has no built-in system files, which come with its own LLVM on macOS and Linux (D50, D51), so it can only link through `lld`",
    ))
}
