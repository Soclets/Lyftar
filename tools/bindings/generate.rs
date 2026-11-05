// How the raw bindings are made, shared by `tools/bindings` (the committed macOS file) and `build.rs` (every other
// platform, D14).

// trace: D12
fn generate_bindings(include_directory: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let mut headers = Vec::new();
    collect_headers(&include_directory.join("llvm-c"), &mut headers)?;
    headers.sort();
    // Bindgen's default Rust target is older than edition 2024, and it refuses the pair, so name the first stable
    // release that has edition 2024.
    let rust_target =
        bindgen::RustTarget::stable(85, 0).map_err(|error| format!("bindgen does not know Rust 1.85: {error}"))?;
    let mut builder = bindgen::Builder::default()
        .clang_arg(format!("-I{}", include_directory.display()))
        // Either separator, since Windows paths use a backslash.
        .allowlist_file(r".*[/\\]llvm-c[/\\].*")
        .default_enum_style(bindgen::EnumVariation::NewType { is_bitfield: false, is_global: false })
        .rust_target(rust_target)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .layout_tests(false)
        .generate_comments(false)
        .formatter(bindgen::Formatter::None);
    for header in &headers {
        builder = builder.header(header.display().to_string());
    }
    let bindings = builder.generate().map_err(|error| format!("bindgen could not read LLVM's C headers: {error}"))?;
    bindings.write_to_file(output).map_err(|error| format!("could not write {}: {error}", output.display()))
}

fn collect_headers(directory: &std::path::Path, headers: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let read_error = |error: std::io::Error| format!("could not read {}: {error}", directory.display());
    for entry in std::fs::read_dir(directory).map_err(read_error)? {
        let path = entry.map_err(read_error)?.path();
        if path.is_dir() {
            collect_headers(&path, headers)?;
        } else if path.extension().is_some_and(|extension| extension == "h") {
            headers.push(path);
        }
    }
    Ok(())
}
