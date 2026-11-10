// Finds LLVM, downloading Lyftar's static one unless told where one is (D50), checks it is the one version Lyftar supports
// (D2, D13), links it, compiles the C++ shim (D16), and on anything but macOS generates the bindings (D14).

use std::path::{Path, PathBuf};
use std::process::Command;

// trace: D2
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
    // Lyftar's tests read objects with the tools beside the LLVM this build links.
    println!("cargo:rustc-env=LYFTAR_LLVM_CONFIG_IN_USE={}", llvm_config.display());
    // trace: D2, D13
    let version = run(&llvm_config, &["--version"])?;
    if version.split('.').next() != Some(LLVM_MAJOR) {
        return Err(format!("Lyftar supports LLVM {LLVM_MAJOR} only, and {} is LLVM {version}", llvm_config.display()));
    }

    // Windows' official LLVM is built with the static C runtime. A program with the other one still links, then crashes when
    // it frees what LLVM allocated, so this stops here instead.
    let is_msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    let has_static_runtime = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .is_ok_and(|features| features.split(',').any(|feature| feature == "crt-static"));
    if is_msvc && !has_static_runtime {
        return Err("on Windows Lyftar needs the static C runtime that LLVM is built with: build with \
                    RUSTFLAGS=\"-C target-feature=+crt-static\""
            .to_string());
    }

    let library_directory = run(&llvm_config, &["--libdir"])?;
    println!("cargo:rustc-link-search=native={library_directory}");
    // Windows' official download has no shared LLVM, only static libraries, and those need the system's libraries too.
    // With MSVC `--libs` gives full paths, quoted when they have a space, and the directory is already searched.
    let is_static = run(&llvm_config, &["--shared-mode"])? == "static";
    // trace: D53
    // A plugin built with a static LLVM would carry its own copy into a program that has one.
    if is_static {
        println!("cargo:rustc-cfg=lyftar_static_llvm");
    }
    // trace: D52
    // lld comes with Lyftar's static LLVM. An LLVM without its libraries, such as Homebrew's, builds a Lyftar whose `lld` and
    // `link` say so instead. They go before the LLVM libraries they use.
    let lld_libraries = ["lldELF", "lldMachO", "lldCommon"];
    let has_lld = lld_libraries.iter().all(|name| {
        ["a", "so", "dylib", "lib"].iter().any(|extension| {
            let file = if *extension == "lib" { format!("{name}.lib") } else { format!("lib{name}.{extension}") };
            Path::new(&library_directory).join(file).exists()
        })
    });
    if has_lld {
        println!("cargo:rustc-cfg=lyftar_has_lld");
        for name in lld_libraries {
            println!("cargo:rustc-link-lib=dylib={name}");
        }
    }
    let libraries = if is_static {
        run(&llvm_config, &["--link-static", if is_msvc { "--libnames" } else { "--libs" }, "--system-libs"])?
    } else {
        run(&llvm_config, &["--link-shared", "--libs"])?
    };
    // trace: D50
    // A static LLVM from `llvm.yml` has `lto.h` and `Remarks.h`'s C APIs as static libraries too, which go before what they use.
    let has_static_lto = Path::new(&library_directory).join("libLTO.a").exists()
        && Path::new(&library_directory).join("libRemarks.a").exists();
    if has_static_lto {
        println!("cargo:rustc-link-lib=static=LTO");
        println!("cargo:rustc-link-lib=static=Remarks");
    }
    for library in libraries.split_whitespace() {
        // `-lLLVM` on Unix. On Windows `LLVMCore.lib`, or a path on the machine that built LLVM for what it depends on.
        let file_name = library.rsplit(['/', '\\']).next().unwrap_or(library);
        let name = library
            .strip_prefix("-l")
            .or_else(|| file_name.strip_suffix(".lib"))
            .ok_or_else(|| format!("unexpected library from llvm-config: {library}"))?;
        println!("cargo:rustc-link-lib=dylib={name}");
    }
    // LLVM keeps the C API of `lto.h` in a library of its own, and `LLVMRemarkVersion` alone in another. These come last, so
    // the rest of the remark functions resolve to the LLVM linked above.
    if !has_static_lto {
        println!("cargo:rustc-link-lib=dylib=LTO");
        println!("cargo:rustc-link-lib=dylib=Remarks");
    }

    compile_shim(&llvm_config, has_lld)?;
    embed_system_files()?;

    let target = std::env::var("CARGO_CFG_TARGET_OS").map_err(|error| format!("no target from Cargo: {error}"))?;
    if target != "macos" {
        generate(Path::new(&run(&llvm_config, &["--includedir"])?))?;
    }
    Ok(())
}

// trace: D25
fn compile_shim(llvm_config: &Path, has_lld: bool) -> Result<(), String> {
    let is_msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    let mut shim = cc::Build::new();
    shim.cpp(true)
        .file("shim/shim.cpp")
        .file("shim/passes.cpp")
        .file("shim/garbage_collection.cpp")
        .files(has_lld.then_some("shim/link.cpp"))
        .warnings(true)
        .extra_warnings(true)
        .warnings_into_errors(!is_msvc);
    // The same static C runtime `main` checks for.
    if is_msvc {
        shim.static_crt(true);
    }
    // A plugin built with another compiler against Homebrew's libc++ LLVM crashed `opt` (D25), so on macOS the shim is built
    // with the Clang beside the LLVM it links. Lyftar's static LLVM has no Clang, and links the system's libc++, which the
    // system's Clang builds against.
    let beside = Path::new(&run(llvm_config, &["--bindir"])?).join("clang++");
    if cfg!(target_os = "macos") && beside.exists() {
        shim.compiler(beside);
    }
    // llvm-config says -std=c++17, and the shim is C++23 with libc++ hardening on instead.
    for flag in run(llvm_config, &["--cxxflags"])?.split_whitespace() {
        if let Some(directory) = flag.strip_prefix("-I") {
            // LLVM's own headers do not build under the shim's warnings.
            shim.flag(if is_msvc { "/imsvc" } else { "-isystem" }).flag(directory);
        } else if !flag.starts_with("-std=") && !flag.starts_with("-std:") {
            shim.flag(flag);
        }
    }
    shim.flag(if is_msvc { "/std:c++latest" } else { "-std=c++23" });
    shim.define("_LIBCPP_HARDENING_MODE", "_LIBCPP_HARDENING_MODE_FAST");
    // LLVM's inline code keeps its assertions in a debug build of the shim, which catches a broken contract early.
    if std::env::var("PROFILE").is_ok_and(|profile| profile == "release") {
        shim.define("NDEBUG", None);
    }
    // trace: D27
    if std::env::var_os("LYFTAR_SANITIZE").is_some() {
        shim.flag("-fsanitize=address,undefined").flag("-fno-sanitize-recover=all").flag("-fno-omit-frame-pointer");
        // LLVM's library isn't instrumented, so when it grows a `std::vector` the shim made, ASan's marks on that vector go
        // stale and report reads inside the allocation. Only those marks go; a read past the allocation is still caught.
        shim.define("__SANITIZER_DISABLE_CONTAINER_OVERFLOW__", None);
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

// trace: D50
/// The LLVM version Lyftar's published static LLVM and lld are, and each host's file's SHA-256, from the release `llvm.yml` made.
const PUBLISHED_VERSION: &str = "23.1.2";
const PUBLISHED: &[(&str, &str)] = &[
    ("aarch64-apple-darwin", "fdd0b1238955ec34dcb5a19f9c4bca5e9783e4c3ec5fe7ea36e4d923ffc7793e"),
    ("aarch64-unknown-linux-gnu", "6c5f331f3d60f5272f3024d5e7e254d9519ec2a0520e6807e8812d35a31026a0"),
    ("x86_64-unknown-linux-gnu", "0d1aaf0ff83fe655df6f693e0d04c68d030b58660532237aa6c0eb530da850fe"),
];
/// The SHA-256 of the system files' file, the same for every host.
const PUBLISHED_SYSTEM_FILES: &str = "";

/// `LYFTAR_LLVM_CONFIG` if it is set, and otherwise the published static LLVM and lld for this host, downloaded once into a cache
/// and checked against the hash above (D50).
fn find_llvm_config() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("LYFTAR_LLVM_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    let target = std::env::var("TARGET").map_err(|error| format!("no TARGET from Cargo: {error}"))?;
    let Some((_, hash)) = PUBLISHED.iter().find(|(host, _)| *host == target) else {
        return Err(format!(
            "Lyftar has no static LLVM {PUBLISHED_VERSION} published for {target}; set LYFTAR_LLVM_CONFIG to an llvm-config for LLVM {LLVM_MAJOR}"
        ));
    };
    let unpacked = download(&format!("lyftar-llvm-{PUBLISHED_VERSION}-{target}"), hash)?;
    Ok(unpacked.join("bin/llvm-config"))
}

/// A published file of this name, downloaded once into the cache, checked against its hash, and unpacked. Gives the folder it
/// unpacked into, which has the same name.
fn download(name: &str, hash: &str) -> Result<PathBuf, String> {
    let cache = cache_directory()?;
    let unpacked = cache.join(name);
    let marker = unpacked.join(".sha256");
    if std::fs::read_to_string(&marker).is_ok_and(|found| found == hash) {
        return Ok(unpacked);
    }
    std::fs::create_dir_all(&cache).map_err(|error| format!("{}: {error}", cache.display()))?;
    // Each build works in a place of its own, so two at once don't meet, and only a checked copy is moved into place.
    let work = cache.join(format!("{name}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| format!("{}: {error}", work.display()))?;
    let archive = work.join(format!("{name}.tar.xz"));
    let url = format!("https://github.com/Soclets/Lyftar/releases/download/llvm-{PUBLISHED_VERSION}/{name}.tar.xz");
    let fetched = Command::new("curl")
        .args(["--fail", "--location", "--silent", "--show-error", "--output"])
        .arg(&archive)
        .arg(&url)
        .status()
        .map_err(|error| format!("running curl to download {url}: {error}"))?;
    if !fetched.success() {
        return Err(format!(
            "could not download {url}; set LYFTAR_LLVM_CONFIG to use an LLVM {LLVM_MAJOR} of your own"
        ));
    }
    let bytes = std::fs::read(&archive).map_err(|error| format!("{}: {error}", archive.display()))?;
    let found = sha256_hex(&bytes);
    if found != hash {
        let _ = std::fs::remove_dir_all(&work);
        return Err(format!("{url} has SHA-256 {found}, and Lyftar expects {hash}, so it wasn't used"));
    }
    let unpacked_here = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&work)
        .status()
        .map_err(|error| format!("running tar on {}: {error}", archive.display()))?;
    if !unpacked_here.success() {
        return Err(format!("tar could not unpack {}", archive.display()));
    }
    std::fs::write(work.join(name).join(".sha256"), hash).map_err(|error| format!("{}: {error}", work.display()))?;
    let _ = std::fs::remove_dir_all(&unpacked);
    // Another build may have put its own checked copy there first, which is as good.
    if std::fs::rename(work.join(name), &unpacked).is_err() && !marker.exists() {
        return Err(format!("could not move {} into {}", work.display(), unpacked.display()));
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(unpacked)
}

// trace: D51, D52
/// The system files each target needs to link: the published ones, built into Lyftar so a compiler carries them, or none if
/// they aren't published yet, in which case `link` says so.
fn embed_system_files() -> Result<(), String> {
    if PUBLISHED_SYSTEM_FILES.is_empty() {
        return Ok(());
    }
    let folder = download(&format!("lyftar-system-{PUBLISHED_VERSION}"), PUBLISHED_SYSTEM_FILES)?;
    let mut entries = String::from("pub(crate) static FILES: &[(&str, &[u8])] = &[\n");
    for (directory, files) in SYSTEM_FILES {
        for file in *files {
            let path = folder.join(directory).join(file);
            if !path.exists() {
                return Err(format!("{} is missing from the system files", path.display()));
            }
            entries.push_str(&format!(
                "    (\"{directory}/{file}\", include_bytes!({:?})),\n",
                path.display().to_string()
            ));
        }
    }
    entries.push_str("];\n");
    let out_directory =
        PathBuf::from(std::env::var("OUT_DIR").map_err(|error| format!("no OUT_DIR from Cargo: {error}"))?);
    std::fs::write(out_directory.join("system_files.rs"), entries)
        .map_err(|error| format!("writing system_files.rs: {error}"))?;
    println!("cargo:rustc-cfg=lyftar_has_system_files");
    Ok(())
}

/// What `llvm.yml` puts in the system files, by folder.
const SYSTEM_FILES: &[(&str, &[&str])] = &[
    ("linux-aarch64", &["crt1.o", "crti.o", "crtn.o", "libc.a", "libclang_rt.builtins.a"]),
    ("linux-x86_64", &["crt1.o", "crti.o", "crtn.o", "libc.a", "libclang_rt.builtins.a"]),
    ("cortex-m4", &["libclang_rt.builtins.a"]),
];

/// Where downloads are kept between builds and projects: the user's cache directory.
fn cache_directory() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home.map(|home| home.join(".cache")))
    };
    base.map(|base| base.join("lyftar"))
        .ok_or_else(|| "no HOME to keep the downloaded LLVM in; set LYFTAR_LLVM_CONFIG".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
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
