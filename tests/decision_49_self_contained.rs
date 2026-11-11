//! D49's test: a compiler built on Lyftar compiles C for macOS, for Linux on ARM and x86-64 and for a Cortex-M4, and links a hello
//! world for each of the first three and a Cortex-M4 image from a linker script, in one process, while it can't read any LLVM,
//! lld, clang, SDK or developer tools. On macOS `sandbox-exec` keeps it from them. The programs then run: macOS's here, and
//! Linux's where a Linux to run them is (D49, D50, D51, D52, D55).

#![cfg(all(lyftar_has_lld, lyftar_has_system_files, lyftar_has_clang, not(target_os = "windows")))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn llvm_tool(name: &str) -> PathBuf {
    let binaries = Command::new(env!("LYFTAR_LLVM_CONFIG_IN_USE")).arg("--bindir").output().expect("llvm-config");
    Path::new(String::from_utf8_lossy(&binaries.stdout).trim()).join(name)
}

/// What this machine has that a compiler could have leaned on, which the sandbox denies: the LLVM this build used, Homebrew, the
/// downloaded files, Xcode and the command line tools.
fn denied() -> Vec<PathBuf> {
    let prefix = Command::new(env!("LYFTAR_LLVM_CONFIG_IN_USE")).arg("--prefix").output().expect("llvm-config");
    let mut paths = vec![
        PathBuf::from(String::from_utf8_lossy(&prefix.stdout).trim()),
        PathBuf::from("/opt/homebrew"),
        PathBuf::from("/usr/local"),
        PathBuf::from("/Applications/Xcode.app"),
        PathBuf::from("/Library/Developer"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(Path::new(&home).join("Library/Caches/lyftar"));
        paths.push(Path::new(&home).join(".cache/lyftar"));
    }
    paths
}

fn build_the_compiler() -> PathBuf {
    let built = Command::new(env!("CARGO"))
        .args(["build", "--example", "self_contained"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    path.pop();
    path.join("examples/self_contained")
}

#[test]
fn decision_49_a_compiler_on_lyftar_links_for_every_target_with_nothing_installed() {
    let compiler = build_the_compiler();
    // Under the target folder, which is under the home folder, so a Linux virtual machine sharing it can see the programs.
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("self-contained-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let mut run = if cfg!(target_os = "macos") {
        let rules: String = denied()
            .iter()
            .map(|path| {
                format!(" (subpath {:?})", path.canonicalize().unwrap_or_else(|_| path.clone()).display().to_string())
            })
            .collect();
        let profile = format!(
            "(version 1)(allow default)(deny file-read*{rules})\
             (deny process-exec (literal \"/usr/bin/ld\") (literal \"/usr/bin/clang\") (literal \"/usr/bin/xcrun\"))"
        );
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(profile).arg(&compiler);
        command
    } else {
        Command::new(&compiler)
    };
    let ran = run.arg(&folder).env("PATH", "/nonexistent").output().expect("running the compiler");
    assert!(ran.status.success(), "{}{}", String::from_utf8_lossy(&ran.stdout), String::from_utf8_lossy(&ran.stderr));

    // What the compiler links: only the system's own libraries, never LLVM's.
    let linked = if cfg!(target_os = "macos") {
        Command::new("otool").arg("-L").arg(&compiler).output()
    } else {
        Command::new("ldd").arg(&compiler).output()
    }
    .expect("listing what it links");
    let linked = String::from_utf8_lossy(&linked.stdout).to_lowercase();
    assert!(!linked.contains("libllvm") && !linked.contains("liblto") && !linked.contains("liblld"), "{linked}");

    // The macOS program runs on this Mac, in the same sandbox.
    if cfg!(target_os = "macos") {
        let hello = Command::new(folder.join("hello-macos")).output().expect("running it");
        assert!(hello.status.success());
        assert_eq!(String::from_utf8_lossy(&hello.stdout), "hello from a compiler built on Lyftar, hello-macos, 49\n");
    }
    // The Linux programs are standalone, so this machine's Linux runs one natively.
    let native = if cfg!(target_arch = "x86_64") { "hello-linux-x86_64" } else { "hello-linux-aarch64" };
    if cfg!(target_os = "linux") {
        let hello = Command::new(folder.join(native)).output().expect("running it");
        assert_eq!(
            String::from_utf8_lossy(&hello.stdout),
            format!("hello from a compiler built on Lyftar, {native}, 49\n")
        );
    }
    // And where Docker has a Linux on ARM, it runs there too: on this Mac, under colima.
    let docker = Command::new("docker").args(["info", "--format", "{{.Architecture}}"]).output();
    if let Ok(docker) = docker
        && docker.status.success()
        && String::from_utf8_lossy(&docker.stdout).trim() == "aarch64"
    {
        let hello = Command::new("docker")
            .args(["run", "--rm", "-v"])
            .arg(format!("{}:/programs:ro", folder.display()))
            .args(["alpine:3", "/programs/hello-linux-aarch64"])
            .output()
            .expect("running Docker");
        assert_eq!(
            String::from_utf8_lossy(&hello.stdout),
            "hello from a compiler built on Lyftar, hello-linux-aarch64, 49\n",
            "{}",
            String::from_utf8_lossy(&hello.stderr)
        );
        eprintln!("ran hello-linux-aarch64 in Docker");
    }
    for name in ["hello-linux-aarch64", "hello-linux-x86_64"] {
        let headers = Command::new(llvm_tool("llvm-readobj"))
            .arg("--file-headers")
            .arg("--program-headers")
            .arg(folder.join(name))
            .output();
        let headers = String::from_utf8_lossy(&headers.expect("llvm-readobj").stdout).into_owned();
        assert!(headers.contains("Type: Executable"), "{name}: {headers}");
        assert!(!headers.contains("PT_INTERP") && !headers.contains("PT_DYNAMIC"), "{name} is standalone: {headers}");
    }

    // The Cortex-M4 image: its vector table first in flash, pointing at the reset handler, which is its entry.
    for name in ["cortex-m4.elf", "cortex-m4-again.elf"] {
        let image = folder.join(name);
        let headers =
            Command::new(llvm_tool("llvm-readobj")).arg("--file-headers").arg(&image).output().expect("llvm-readobj");
        let headers = String::from_utf8_lossy(&headers.stdout).into_owned();
        assert!(headers.contains("Machine: EM_ARM"), "{headers}");
        let symbols =
            Command::new(llvm_tool("llvm-objdump")).arg("--syms").arg("--section-headers").arg(&image).output();
        let symbols = String::from_utf8_lossy(&symbols.expect("llvm-objdump").stdout).into_owned();
        let address = |name: &str| -> u64 {
            let line = symbols
                .lines()
                .find(|line| line.ends_with(&format!(" {name}")))
                .unwrap_or_else(|| panic!("{name}: {symbols}"));
            u64::from_str_radix(line.split_whitespace().next().expect("an address"), 16).expect("hexadecimal")
        };
        let section = |section: &str| {
            symbols.lines().find(|line| line.split_whitespace().nth(1) == Some(section)).map(str::to_string)
        };
        let vectors = section(".isr_vector").unwrap_or_else(|| panic!("a vector table: {symbols}"));
        assert!(vectors.contains("08000000"), "{vectors}");
        let reset = address("reset");
        // Thumb code: the entry and the vector table's reset entry are its address with the lowest bit set.
        assert!(headers.contains(&format!("Entry: 0x{:X}", reset | 1)), "{headers}");
        let contents = Command::new(llvm_tool("llvm-objdump")).args(["-s", "-j", ".isr_vector"]).arg(&image).output();
        let contents = String::from_utf8_lossy(&contents.expect("llvm-objdump").stdout).into_owned();
        let reset_entry = u32::try_from(reset | 1).expect("a 32-bit address").to_le_bytes();
        let hex: String = reset_entry.iter().map(|byte| format!("{byte:02x}")).collect();
        assert!(contents.contains(&format!("00000120 {hex}")), "{contents}");
    }
    let _ = std::fs::remove_dir_all(&folder);
}
