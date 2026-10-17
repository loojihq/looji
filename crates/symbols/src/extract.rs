//! Parses one file with tree-sitter and reads its symbols out with the language's query.

use std::collections::HashMap;

use looji_domain::{Language, Symbol, SymbolKind};
use streaming_iterator::StreamingIterator;
use thiserror::Error;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::queries;

/// Why a file produced no symbols. Each one is logged and recorded, and never stops the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The file couldn't be read.
    Unreadable(String),
    /// The file isn't UTF-8 text.
    NotUtf8,
    /// The file is larger than the limit, so it isn't parsed.
    TooLarge { bytes: u64 },
    /// The parser found a syntax error. `line` is where the first one is.
    SyntaxError { line: u32 },
    /// The parser gave up without a tree.
    ParserGaveUp,
    /// The grammar couldn't be set on the parser. That means a grammar and tree-sitter disagree, not a bad file.
    Grammar(String),
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Unreadable(reason) => write!(f, "could not be read ({reason})"),
            SkipReason::NotUtf8 => write!(f, "is not UTF-8 text"),
            SkipReason::TooLarge { bytes } => write!(f, "is {bytes} bytes, over the size limit"),
            SkipReason::SyntaxError { line } => write!(f, "has a syntax error near line {line}"),
            SkipReason::ParserGaveUp => write!(f, "the parser stopped before it finished"),
            SkipReason::Grammar(reason) => write!(f, "the grammar could not be used ({reason})"),
        }
    }
}

/// A grammar or query that didn't load. This is a bug in looji, not in the file being read.
#[derive(Debug, Error)]
pub enum ExtractorError {
    #[error("the grammar for {language} didn't load: {reason}")]
    Grammar {
        language: &'static str,
        reason: String,
    },
    #[error("the symbol query for {language} is invalid: {reason}")]
    Query {
        language: &'static str,
        reason: String,
    },
}

/// Holds a parser and the compiled queries, so a whole index compiles each one once.
pub struct Extractor {
    parser: Parser,
    queries: HashMap<Language, Query>,
}

impl Extractor {
    pub fn new() -> Result<Self, ExtractorError> {
        let mut queries = HashMap::new();
        for language in ALL_LANGUAGES {
            let grammar = grammar(language);
            let query = Query::new(&grammar, query_source(language)).map_err(|err| {
                ExtractorError::Query {
                    language: language.name(),
                    reason: err.to_string(),
                }
            })?;
            queries.insert(language, query);
        }
        Ok(Self {
            parser: Parser::new(),
            queries,
        })
    }

    /// The symbols in one file. `path` is only copied into each symbol, so it should be relative to the repository.
    pub fn extract(
        &mut self,
        language: Language,
        path: &str,
        source: &str,
    ) -> Result<Vec<Symbol>, SkipReason> {
        self.parser
            .set_language(&grammar(language))
            .map_err(|err| SkipReason::Grammar(err.to_string()))?;
        let tree = self
            .parser
            .parse(source, None)
            .ok_or(SkipReason::ParserGaveUp)?;
        let root = tree.root_node();
        if root.has_error() {
            let line = first_error_line(root).unwrap_or(1);
            return Err(SkipReason::SyntaxError { line });
        }
        Ok(self.symbols(language, path, source, root))
    }

    fn symbols(&self, language: Language, path: &str, source: &str, root: Node) -> Vec<Symbol> {
        let query = &self.queries[&language];
        let names = query.capture_names();
        let bytes = source.as_bytes();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, root, bytes);
        let mut symbols = Vec::new();
        while let Some(found) = matches.next() {
            let mut name = None;
            let mut definition = None;
            for capture in found.captures() {
                let capture_name = names[capture.index as usize];
                if capture_name == "name" {
                    name = capture.node.utf8_text(bytes).ok().map(str::to_owned);
                } else if let Some(kind) = capture_name.strip_prefix("definition.") {
                    definition = Some((kind, capture.node));
                }
            }
            let (Some(name), Some((kind, node))) = (name, definition) else {
                continue;
            };
            let Some(kind) = symbol_kind(kind) else {
                continue;
            };
            symbols.push(Symbol {
                name,
                kind,
                path: path.to_string(),
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
            });
        }
        symbols
    }
}

const ALL_LANGUAGES: [Language; 6] = [
    Language::Rust,
    Language::Python,
    Language::JavaScript,
    Language::TypeScript,
    Language::Tsx,
    Language::Go,
];

fn grammar(language: Language) -> tree_sitter::Language {
    match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
    }
}

fn query_source(language: Language) -> &'static str {
    match language {
        Language::Rust => queries::RUST,
        Language::Python => queries::PYTHON,
        Language::JavaScript => queries::JAVASCRIPT,
        Language::TypeScript | Language::Tsx => queries::TYPESCRIPT,
        Language::Go => queries::GO,
    }
}

fn symbol_kind(kind: &str) -> Option<SymbolKind> {
    match kind {
        "function" => Some(SymbolKind::Function),
        "type" => Some(SymbolKind::Type),
        "module" => Some(SymbolKind::Module),
        _ => None,
    }
}

/// The line of the first error or missing token. Subtrees without errors are skipped.
fn first_error_line(node: Node) -> Option<u32> {
    if node.is_error() || node.is_missing() {
        return Some(node.start_position().row as u32 + 1);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_error_line)
}
