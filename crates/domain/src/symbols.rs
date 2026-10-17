//! What a symbol is, which languages are read, and the order symbols come back in. Pure. The parsing is in the
//! symbols crate.

/// A language whose files are read for symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    /// TypeScript with JSX, which needs its own grammar.
    Tsx,
    Go,
}

impl Language {
    /// The language for a file, from its extension. Case doesn't matter. `None` for files that aren't read.
    pub fn from_path(path: &str) -> Option<Self> {
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match extension.as_str() {
            "rs" => Language::Rust,
            "py" => Language::Python,
            "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "go" => Language::Go,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Go => "Go",
        }
    }
}

/// The kinds of symbol the index records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A function, a method, or a declared signature of one.
    Function,
    /// A struct, class, interface, enum, trait, or type alias.
    Type,
    /// A module, namespace, or package declaration.
    Module,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Type => "type",
            SymbolKind::Module => "module",
        }
    }
}

/// A named definition in a file. Lines are 1-based and inclusive at both ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The file, relative to the repository root, with `/` between parts.
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// Orders symbols by file, then by where they start in it, then by name. The same symbols always come back in the
/// same order.
pub fn sort_symbols(symbols: &mut [Symbol]) {
    symbols.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.start_line.cmp(&b.start_line))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| (a.kind as u8).cmp(&(b.kind as u8)))
    });
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::symbols::{Language, Symbol, SymbolKind, sort_symbols};

    #[test]
    fn languages_are_recognised_by_extension_in_any_case() {
        assert_eq!(Language::from_path("src/lib.rs"), Some(Language::Rust));
        assert_eq!(Language::from_path("tool.PY"), Some(Language::Python));
        assert_eq!(Language::from_path("app.jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_path("api.ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_path("view.tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_path("main.go"), Some(Language::Go));
    }

    #[test]
    fn other_files_are_not_read() {
        assert_eq!(Language::from_path("README.md"), None);
        assert_eq!(Language::from_path("Makefile"), None);
        assert_eq!(Language::from_path("archive.rs.bak"), None);
    }

    fn symbol(name: &str, path: &str, start_line: u32) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            path: path.to_string(),
            start_line,
            end_line: start_line,
        }
    }

    #[test]
    fn symbols_are_ordered_by_file_then_line_then_name() {
        let mut symbols = vec![
            symbol("zeta", "b.rs", 1),
            symbol("beta", "a.rs", 9),
            symbol("alpha", "a.rs", 9),
            symbol("early", "a.rs", 2),
        ];
        sort_symbols(&mut symbols);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["early", "alpha", "beta", "zeta"]);
    }
}
