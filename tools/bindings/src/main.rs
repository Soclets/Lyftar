// Regenerates the committed macOS bindings when LLVM moves (D13):
//   cargo run --manifest-path tools/bindings/Cargo.toml -- <llvm include directory> src/raw/macos.rs

include!("../generate.rs");

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let usage = "usage: lyftar-bindings <llvm include directory> <output file>";
    let include_directory = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;
    generate_bindings(std::path::Path::new(&include_directory), std::path::Path::new(&output))?;
    // Formatted, so that moving LLVM shows as a readable diff of the C API (D13).
    let formatted = std::process::Command::new("rustfmt").args(["--edition", "2024", &output]).status();
    match formatted {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("rustfmt failed on {output}: {status}")),
        Err(error) => Err(format!("could not run rustfmt: {error}")),
    }
}
