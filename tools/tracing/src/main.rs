//! The tracing check from `IMPLEMENTATION.md`: fails on a `trace:` tag that names no decision, and lists the decisions nothing
//! traces yet. `cargo run --manifest-path tools/tracing/Cargo.toml -- <repository>`.

// trace: D23
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SOURCE_EXTENSIONS: [&str; 5] = ["rs", "cpp", "h", "toml", "yml"];
const SKIPPED_DIRECTORIES: [&str; 3] = ["target", ".git", "compile_fail"];

fn source_files(directory: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| format!("{}: {error}", directory.display()))?.path();
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if path.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                source_files(&path, found)?;
            }
        } else if path.extension().is_some_and(|extension| SOURCE_EXTENSIONS.iter().any(|known| extension == *known)) {
            found.push(path);
        }
    }
    Ok(())
}

/// The decision numbers in `DECISIONS.md`, each of which starts a line as `**D12. `.
fn decisions(text: &str) -> BTreeSet<u32> {
    text.lines()
        .filter_map(|line| line.strip_prefix("**D")?.split_once(". ")?.0.parse().ok())
        .collect()
}

/// The tags on one line: one decision, or several with commas between them.
fn tags(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    // Spelled apart so that this file doesn't read as holding a tag.
    for (start, marker) in line.match_indices(concat!("trace", ": D")) {
        // The same words at the end of a longer word aren't a tag.
        if line[..start].chars().next_back().is_some_and(char::is_alphanumeric) {
            continue;
        }
        let rest = &line[start + marker.len() - 1..];
        for tag in rest.split(',') {
            let tag = tag.trim();
            let end = tag.find(|character: char| character.is_whitespace()).unwrap_or(tag.len());
            found.push(tag[..end].trim_end_matches(['.', ')', ';']).to_string());
            if end != tag.len() {
                break;
            }
        }
    }
    found
}

fn check(repository: &Path) -> Result<(Vec<String>, BTreeSet<u32>), String> {
    let path = repository.join("DECISIONS.md");
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let known = decisions(&text);
    let mut files = Vec::new();
    source_files(repository, &mut files)?;
    let mut bad = Vec::new();
    let mut traced = BTreeSet::new();
    for file in files {
        // A file that isn't text holds no tags.
        let Ok(source) = fs::read_to_string(&file) else { continue };
        for (index, line) in source.lines().enumerate() {
            for tag in tags(line) {
                match tag.strip_prefix('D').and_then(|number| number.parse::<u32>().ok()) {
                    Some(number) if known.contains(&number) => {
                        traced.insert(number);
                    }
                    _ => bad.push(format!("{}:{}: {tag} names no decision", file.display(), index + 1)),
                }
            }
        }
    }
    Ok((bad, known.difference(&traced).copied().collect()))
}

fn main() -> ExitCode {
    let repository = std::env::args().nth(1).map_or_else(|| PathBuf::from("."), PathBuf::from);
    match check(&repository) {
        Ok((bad, untraced)) => {
            let names: Vec<String> = untraced.iter().map(|number| format!("D{number}")).collect();
            println!("nothing traces yet: {}", if names.is_empty() { "none".to_string() } else { names.join(", ") });
            for line in &bad {
                println!("{line}");
            }
            if bad.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Err(message) => {
            println!("{message}");
            ExitCode::FAILURE
        }
    }
}
