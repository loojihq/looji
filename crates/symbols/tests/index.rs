//! Symbol maps against the fixture repository in `tests/fixtures/repo`, and against folders built for the edge cases.
//! The fixture's counts were worked out from its files. Each language has one good file, and a broken file that
//! must be skipped.

use std::{
    fs,
    path::{Path, PathBuf},
};

use looji_domain::{Language, Symbol, SymbolKind};
use looji_persistence::file_tree::scan_repo_tree;
use looji_symbols::{MAX_FILE_BYTES, SkipReason, SymbolIndex, index_tree};
use tempfile::TempDir;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/repo")
}

fn index_of(root: &Path) -> SymbolIndex {
    let tree = scan_repo_tree(root).unwrap();
    index_tree(root, &tree.roots).unwrap()
}

fn count(index: &SymbolIndex, path: &str, kind: SymbolKind) -> usize {
    index
        .symbols
        .iter()
        .filter(|symbol| symbol.path == path && symbol.kind == kind)
        .count()
}

fn named<'a>(index: &'a SymbolIndex, path: &str, name: &str) -> &'a Symbol {
    index
        .symbols
        .iter()
        .find(|symbol| symbol.path == path && symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol {name} in {path}"))
}

#[test]
fn each_supported_language_gives_the_expected_symbol_counts() {
    let index = index_of(&fixture_root());
    // (file, functions, types, modules)
    let expected = [
        ("go/shapes.go", 3, 3, 1),
        ("javascript/app.js", 3, 1, 0),
        ("python/shapes.py", 4, 2, 0),
        ("rust/geometry.rs", 5, 4, 1),
        ("typescript/service.ts", 4, 4, 1),
        ("typescript/view.tsx", 2, 1, 0),
    ];
    for (path, functions, types, modules) in expected {
        assert_eq!(
            count(&index, path, SymbolKind::Function),
            functions,
            "functions in {path}"
        );
        assert_eq!(
            count(&index, path, SymbolKind::Type),
            types,
            "types in {path}"
        );
        assert_eq!(
            count(&index, path, SymbolKind::Module),
            modules,
            "modules in {path}"
        );
    }
    assert_eq!(index.symbols.len(), 39, "{:#?}", index.symbols);
}

#[test]
fn every_language_is_represented_in_the_fixture() {
    let index = index_of(&fixture_root());
    let languages: Vec<Language> = index
        .symbols
        .iter()
        .filter_map(|symbol| Language::from_path(&symbol.path))
        .collect();
    for language in [
        Language::Rust,
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Tsx,
        Language::Go,
    ] {
        assert!(
            languages.contains(&language),
            "no symbols for {}",
            language.name()
        );
    }
}

#[test]
fn ranges_cover_the_whole_definition_with_1_based_lines() {
    let index = index_of(&fixture_root());
    let distance = named(&index, "rust/geometry.rs", "distance");
    assert_eq!((distance.start_line, distance.end_line), (33, 35));
    assert_eq!(distance.kind, SymbolKind::Function);

    let point = named(&index, "rust/geometry.rs", "Point");
    assert_eq!((point.start_line, point.end_line), (7, 10));
    assert_eq!(point.kind, SymbolKind::Type);

    let shapes = named(&index, "rust/geometry.rs", "shapes");
    assert_eq!((shapes.start_line, shapes.end_line), (1, 5));
    assert_eq!(shapes.kind, SymbolKind::Module);

    let area = named(&index, "rust/geometry.rs", "area");
    assert_eq!(area.start_line, 18, "the trait's declared method");
}

#[test]
fn a_method_inside_a_class_is_a_function_of_its_file() {
    let index = index_of(&fixture_root());
    let load = named(&index, "javascript/app.js", "load");
    assert_eq!(load.kind, SymbolKind::Function);
    let format = named(&index, "javascript/app.js", "format");
    assert_eq!(
        format.kind,
        SymbolKind::Function,
        "an arrow function bound to a name"
    );
    assert!(
        !index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "notAFunction"),
        "a constant that isn't a function is not a symbol"
    );
}

#[test]
fn the_broken_file_of_each_language_is_skipped_with_its_error_line() {
    let index = index_of(&fixture_root());
    assert_eq!(index.files_indexed, 6, "the six good files");
    let skipped: Vec<(&str, &SkipReason)> = index
        .skipped
        .iter()
        .map(|skip| (skip.path.as_str(), &skip.reason))
        .collect();
    assert_eq!(skipped.len(), 5, "{skipped:?}");

    let line = |path: &str| -> u32 {
        match skipped
            .iter()
            .find(|(skipped_path, _)| *skipped_path == path)
        {
            Some((_, SkipReason::SyntaxError { line })) => *line,
            other => panic!("{path} should be a syntax error, got {other:?}"),
        }
    };
    // Each file's broken line, from the fixture. The parser reports the error on that line or the one after it,
    // since that's where it first can't continue.
    for (path, broken_line) in [
        ("rust/broken.rs", 5),
        ("python/broken.py", 4),
        ("javascript/broken.js", 4),
        ("typescript/broken.ts", 4),
        ("go/broken.go", 5),
    ] {
        let reported = line(path);
        assert!(
            (broken_line..=broken_line + 1).contains(&reported),
            "{path}: broken at line {broken_line}, reported {reported}"
        );
    }
}

#[test]
fn a_broken_file_contributes_no_symbols_not_even_the_valid_ones_before_the_error() {
    let index = index_of(&fixture_root());
    assert!(
        !index
            .symbols
            .iter()
            .any(|symbol| symbol.path.ends_with("broken.rs"))
    );
    assert!(!index.symbols.iter().any(|symbol| symbol.name == "fine"));
}

#[test]
fn notes_and_other_files_are_not_read_at_all() {
    let index = index_of(&fixture_root());
    assert!(!index.skipped.iter().any(|skip| skip.path == "notes.md"));
    assert!(!index.symbols.iter().any(|symbol| symbol.path == "notes.md"));
}

#[test]
fn the_symbols_come_back_in_a_stable_order() {
    let root = fixture_root();
    let first = index_of(&root);
    let second = index_of(&root);
    assert_eq!(first.symbols, second.symbols);
    let mut sorted = first.symbols.clone();
    looji_domain::sort_symbols(&mut sorted);
    assert_eq!(first.symbols, sorted);
}

#[test]
fn a_file_over_the_size_limit_is_skipped_and_the_rest_still_index() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("good.go"),
        "package good\n\nfunc Kept() {}\n",
    )
    .unwrap();
    let big = vec![b'/'; MAX_FILE_BYTES as usize + 1];
    fs::write(dir.path().join("huge.go"), big).unwrap();

    let index = index_of(dir.path());
    assert!(index.symbols.iter().any(|symbol| symbol.name == "Kept"));
    assert_eq!(index.skipped.len(), 1);
    assert_eq!(index.skipped[0].path, "huge.go");
    assert!(matches!(
        index.skipped[0].reason,
        SkipReason::TooLarge { .. }
    ));
}

#[test]
fn a_file_that_is_not_utf8_is_skipped_and_the_rest_still_index() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("good.py"), "def kept():\n    return 1\n").unwrap();
    fs::write(
        dir.path().join("latin1.py"),
        [0x64, 0x65, 0x66, 0x20, 0xE9, 0x3A],
    )
    .unwrap();

    let index = index_of(dir.path());
    assert!(index.symbols.iter().any(|symbol| symbol.name == "kept"));
    assert_eq!(index.skipped.len(), 1);
    assert_eq!(index.skipped[0].reason, SkipReason::NotUtf8);
}
