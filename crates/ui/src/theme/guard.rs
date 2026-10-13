//! Keeps colour literals out of the UI. Components must read tokens; a literal here fails the build.

use std::{fs, path::Path};

/// Spellings that mean "a colour written in place". `theme` itself is exempt.
const FORBIDDEN: &[&str] = &[
    "rgb(", "rgba(", "hsl(", "hsla(", "white()", "black()", "Rgba {", "Hsla {",
];

/// A hex colour literal such as `0xe81120`. Six digits, so unrelated numbers don't match.
fn has_hex_colour(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.windows(8).any(|window| {
        window[0] == b'0' && window[1] == b'x' && window[2..].iter().all(u8::is_ascii_hexdigit)
    })
}

fn offences_in(file: &Path, source: &str) -> Vec<String> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            FORBIDDEN.iter().any(|pattern| line.contains(pattern)) || has_hex_colour(line)
        })
        .map(|(index, line)| format!("{}:{}: {}", file.display(), index + 1, line.trim()))
        .collect()
}

fn collect_rust_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // The theme module is where colours are allowed to live.
            if path.file_name().is_some_and(|name| name == "theme") {
                continue;
            }
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn ui_code_uses_only_theme_tokens_for_colour() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files);
    assert!(!files.is_empty(), "found no UI source files to check");

    let mut offences = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).expect("UI source is readable");
        offences.extend(offences_in(file, &source));
    }
    assert!(
        offences.is_empty(),
        "colour literals outside the theme module; use a palette role instead:\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_scanner_catches_each_spelling() {
    let file = Path::new("example.rs");
    for line in [
        "rgb(0xe81120)",
        "let c = rgba(0x00000080);",
        "hsla(0.5, 0.2, 0.3, 1.0)",
        "gpui::white()",
        "gpui::black().opacity(0.5)",
        "const RED: u32 = 0xe81120;",
    ] {
        assert!(!offences_in(file, line).is_empty(), "missed: {line}");
    }
}

#[test]
fn the_scanner_ignores_tokens_and_unrelated_numbers() {
    let file = Path::new("example.rs");
    let source = "\
.bg(theme.palette.surface)
.text_color(theme.palette.text_secondary)
.px(px(12.))
let uuid = 0x12;
";
    assert!(offences_in(file, source).is_empty());
}
