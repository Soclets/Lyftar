//! Compiling C with clang inside Lyftar (D55, D57, D58, D60). With an LLVM that has no clang, such as Homebrew's, `compile_c`
//! says so instead.

use std::path::{Path, PathBuf};

use lyftar::compile_c;
use lyftar::compiling::CCompileOptions;
use lyftar::linking::SystemTarget;

/// A folder of this test's own, under the target folder.
fn folder(name: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("compile-c-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("a folder");
    folder
}

/// Writes `source` into the folder as `name`, and compiles it to `name` with `.o` after it.
fn compile(
    folder: &Path,
    name: &str,
    source: &str,
    target: SystemTarget,
    options: &CCompileOptions<'_>,
) -> (PathBuf, Result<String, lyftar::error::Error>) {
    let path = folder.join(name);
    std::fs::write(&path, source).expect("the source");
    let object = folder.join(format!("{name}.o"));
    let result = compile_c(target, &path, &object, options);
    (object, result)
}

#[cfg(lyftar_has_clang)]
fn llvm_tool(name: &str) -> PathBuf {
    let binaries =
        std::process::Command::new(env!("LYFTAR_LLVM_CONFIG_IN_USE")).arg("--bindir").output().expect("llvm-config");
    Path::new(String::from_utf8_lossy(&binaries.stdout).trim()).join(name)
}

#[cfg(lyftar_has_clang)]
fn read_object(arguments: &[&str], object: &Path) -> String {
    let output = std::process::Command::new(llvm_tool("llvm-readobj"))
        .args(arguments)
        .arg(object)
        .output()
        .expect("llvm-readobj");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// clang's own headers, and a 128-bit multiply through C23's `_BitInt`, which CORE-MATH uses on every target, the Cortex-M4's
/// 32-bit processor among them.
#[cfg(lyftar_has_clang)]
const PORTABLE: &str = "#include <stdint.h>\n#include <stddef.h>\n#include <stdarg.h>\n#include <stdbool.h>\n#include <limits.h>\n\
#include <float.h>\n#include <stdalign.h>\n\
typedef unsigned _BitInt(128) u128;\n\
uint64_t high_half(uint64_t a, uint64_t b) { return (uint64_t)(((u128)a * b) >> 64); }\n\
size_t size_of_int(void) { return sizeof(int) * CHAR_BIT; }\n";

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_c_compiles_to_an_object_for_every_target() {
    let folder = folder("every-target");
    for (target, format, architecture) in [
        (SystemTarget::MacOsAarch64, "Mach-O arm64", "aarch64"),
        (SystemTarget::LinuxAarch64, "elf64-littleaarch64", "aarch64"),
        (SystemTarget::LinuxX86_64, "elf64-x86-64", "x86_64"),
        (SystemTarget::CortexM4, "elf32-littlearm", "arm"),
    ] {
        let (object, result) =
            compile(&folder, &format!("{target:?}.c"), PORTABLE, target, &CCompileOptions::default());
        let warnings = result.unwrap_or_else(|error| panic!("{target:?}: {}", error.message()));
        assert_eq!(warnings, "", "{target:?}");
        let headers = read_object(&["--file-headers"], &object);
        assert!(headers.contains(&format!("Format: {format}")), "{target:?}: {headers}");
        assert!(headers.contains(&format!("Arch: {architecture}")), "{target:?}: {headers}");
    }
    // The Cortex-M4's own processor, with its hard-float unit and calling convention.
    let attributes = read_object(&["--arch-specific"], &folder.join("CortexM4.c.o"));
    assert!(attributes.contains("Value: cortex-m4"), "{attributes}");
    assert!(attributes.contains("Description: AAPCS VFP"), "{attributes}");
}

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_no_system_folder_is_searched() {
    let folder = folder("no-system");
    // macOS's SDK has a `stdio.h`, and Linux has one in `/usr/include`: neither is found.
    let (_, result) = compile(
        &folder,
        "stdio.c",
        "#include <stdio.h>\n",
        if cfg!(target_os = "macos") { SystemTarget::MacOsAarch64 } else { SystemTarget::LinuxX86_64 },
        &CCompileOptions::default(),
    );
    let error = result.expect_err("no stdio.h");
    assert!(error.message().contains("'stdio.h' file not found"), "{}", error.message());
}

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_errors_and_warnings_come_back_as_clang_wrote_them() {
    let folder = folder("messages");
    let (object, result) =
        compile(&folder, "broken.c", "int f(void) { return }\n", SystemTarget::CortexM4, &CCompileOptions::default());
    let error = result.expect_err("a syntax error");
    assert!(error.message().contains("broken.c:1:22: error: expected expression"), "{}", error.message());
    assert!(!object.exists());
    let (object, result) = compile(
        &folder,
        "warned.c",
        "#warning \"look here\"\nint f(void) { return 1; }\n",
        SystemTarget::CortexM4,
        &CCompileOptions::default(),
    );
    let warnings = result.expect("a warning, not an error");
    assert!(warnings.contains("warned.c:1:2: warning: \"look here\""), "{warnings}");
    assert!(object.exists());
}

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_a_file_compiles_as_c_whatever_its_name() {
    let folder = folder("any-name");
    let (object, result) = compile(
        &folder,
        "source.txt",
        "int f(void) { return 1; }\n",
        SystemTarget::LinuxAarch64,
        &CCompileOptions::default(),
    );
    result.expect("C");
    assert!(read_object(&["--file-headers"], &object).contains("Format: elf64-littleaarch64"));
}

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_include_folders_are_searched_in_order() {
    let folder = folder("includes");
    for (name, value) in [("first", 1), ("second", 2)] {
        std::fs::create_dir_all(folder.join(name)).expect("a folder");
        std::fs::write(folder.join(name).join("value.h"), format!("#define VALUE {value}\n")).expect("a header");
    }
    let (first, second) = (folder.join("first"), folder.join("second"));
    let source = "#include \"value.h\"\n_Static_assert(VALUE == 1, \"the first folder's\");\n";
    let options = CCompileOptions { include_folders: &[&first, &second], ..CCompileOptions::default() };
    let (_, result) = compile(&folder, "includes.c", source, SystemTarget::CortexM4, &options);
    result.unwrap_or_else(|error| panic!("{}", error.message()));
}

// trace: D58
#[cfg(lyftar_has_clang)]
#[test]
fn decision_58_defines_switch_parts_of_the_code() {
    let folder = folder("defines");
    let source = "#ifndef DEBUGGING\n#error \"DEBUGGING isn't defined\"\n#endif\n\
                  _Static_assert(DEBUGGING == 1, \"a define without a value is 1\");\n\
                  _Static_assert(ANSWER == 42, \"the value given\");\n";
    let options =
        CCompileOptions { defines: &[("DEBUGGING", None), ("ANSWER", Some("42"))], ..CCompileOptions::default() };
    let (_, result) = compile(&folder, "defined.c", source, SystemTarget::MacOsAarch64, &options);
    result.unwrap_or_else(|error| panic!("{}", error.message()));
    let (_, result) = compile(&folder, "undefined.c", source, SystemTarget::MacOsAarch64, &CCompileOptions::default());
    let error = result.expect_err("nothing defined");
    assert!(error.message().contains("DEBUGGING isn't defined"), "{}", error.message());
}

// trace: D57
#[cfg(lyftar_has_clang)]
#[test]
fn decision_57_the_optimization_level_is_the_callers_and_o2_by_default() {
    let folder = folder("optimization");
    let source = "static int helper(int x) { return x + 1; }\nint f(int x) { return helper(x); }\n";
    let symbols = |options: &CCompileOptions<'_>, name: &str| {
        let (object, result) = compile(&folder, name, source, SystemTarget::LinuxX86_64, options);
        result.unwrap_or_else(|error| panic!("{}", error.message()));
        read_object(&["--symbols"], &object)
    };
    // `-O0` keeps the helper as a function of its own; `-O2` inlines it away.
    let unoptimized = symbols(
        &CCompileOptions { optimization: lyftar::passes::OptimizationLevel::Level0, ..CCompileOptions::default() },
        "o0.c",
    );
    assert!(unoptimized.contains("Name: helper"), "{unoptimized}");
    let optimized = symbols(&CCompileOptions::default(), "o2.c");
    assert!(!optimized.contains("Name: helper"), "{optimized}");
    assert!(optimized.contains("Name: f "), "{optimized}");
}

#[cfg(lyftar_has_clang)]
#[test]
fn decision_55_compiles_run_at_once_and_give_the_same_bytes() {
    let folder = folder("at-once");
    let targets = [SystemTarget::MacOsAarch64, SystemTarget::LinuxX86_64, SystemTarget::CortexM4];
    let one_at_a_time: Vec<Vec<u8>> = targets
        .iter()
        .map(|target| {
            let (object, result) =
                compile(&folder, &format!("first-{target:?}.c"), PORTABLE, *target, &CCompileOptions::default());
            result.expect("compiled");
            std::fs::read(object).expect("the object")
        })
        .collect();
    // Three, rule 11's cores.
    let at_once: Vec<Vec<u8>> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|target| {
                let folder = &folder;
                scope.spawn(move || {
                    let (object, result) =
                        compile(folder, &format!("again-{target:?}.c"), PORTABLE, *target, &CCompileOptions::default());
                    result.expect("compiled");
                    std::fs::read(object).expect("the object")
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("a thread")).collect()
    });
    // The file's name goes into an ELF object, so the two names are the same length.
    let strip = |bytes: &Vec<u8>| String::from_utf8_lossy(bytes).replace("again-", "first-").into_bytes();
    for ((alone, together), target) in one_at_a_time.iter().zip(&at_once).zip(targets) {
        assert!(strip(alone) == strip(together), "{target:?}");
    }
}

/// A C program on its own, compiled and linked with Lyftar and run here.
#[cfg(all(lyftar_has_clang, lyftar_has_lld, lyftar_has_system_files, any(target_os = "macos", target_os = "linux")))]
#[test]
fn decision_55_a_c_program_compiled_and_linked_by_lyftar_runs() {
    let folder = folder("program");
    let target = if cfg!(target_os = "macos") {
        SystemTarget::MacOsAarch64
    } else if cfg!(target_arch = "x86_64") {
        SystemTarget::LinuxX86_64
    } else {
        SystemTarget::LinuxAarch64
    };
    let source = "int printf(const char *, ...);\n\
                  typedef unsigned _BitInt(128) u128;\n\
                  int main(void) { u128 big = (u128)1 << 100; printf(\"%d\\n\", (int)(big >> 95)); return 0; }\n";
    let (object, result) = compile(&folder, "program.c", source, target, &CCompileOptions::default());
    result.unwrap_or_else(|error| panic!("{}", error.message()));
    let program = folder.join("program");
    lyftar::link(target, &[&object], &program, None).unwrap_or_else(|error| panic!("{}", error.message()));
    let ran = std::process::Command::new(&program).output().expect("the program");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "32\n");
}

#[cfg(not(lyftar_has_clang))]
#[test]
fn decision_55_compile_c_says_when_this_lyftar_has_no_clang() {
    let folder = folder("no-clang");
    let (_, result) =
        compile(&folder, "f.c", "int f(void) { return 1; }\n", SystemTarget::CortexM4, &CCompileOptions::default());
    let error = result.expect_err("no clang");
    assert!(error.message().contains("no clang libraries"), "{}", error.message());
}

#[test]
#[should_panic(expected = "compile_c: \"A=B\" isn't a macro's name")]
fn decision_3_a_define_whose_name_has_an_equals_sign_panics() {
    let options = CCompileOptions { defines: &[("A=B", None)], ..CCompileOptions::default() };
    let _ = compile_c(SystemTarget::CortexM4, Path::new("f.c"), Path::new("f.o"), &options);
}

#[test]
#[should_panic(expected = "compile_c: \"\" isn't a macro's name")]
fn decision_3_a_define_with_no_name_panics() {
    let options = CCompileOptions { defines: &[("", Some("1"))], ..CCompileOptions::default() };
    let _ = compile_c(SystemTarget::CortexM4, Path::new("f.c"), Path::new("f.o"), &options);
}

#[test]
#[should_panic(expected = "compile_c: \"-DA=\\0\" has a NUL byte inside it")]
fn decision_3_a_define_with_a_nul_panics() {
    let options = CCompileOptions { defines: &[("A", Some("\0"))], ..CCompileOptions::default() };
    let _ = compile_c(SystemTarget::CortexM4, Path::new("f.c"), Path::new("f.o"), &options);
}

#[cfg(unix)]
#[test]
#[should_panic(expected = "isn't valid UTF-8, which clang's arguments have to be")]
fn decision_3_a_c_path_that_is_not_text_panics() {
    use std::os::unix::ffi::OsStrExt;
    let source = Path::new(std::ffi::OsStr::from_bytes(b"source\xff.c"));
    let _ = compile_c(SystemTarget::CortexM4, source, Path::new("f.o"), &CCompileOptions::default());
}
