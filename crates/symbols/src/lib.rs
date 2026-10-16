//! Symbol maps: the functions, types, and modules in a repository's source files, with their file and line ranges.
//!
//! Parsing uses tree-sitter. The languages are listed in `looji_domain::Language`. Files that can't be parsed are
//! skipped, logged, and recorded in the index, and the rest of the index still builds.

mod extract;
mod index;
mod queries;

pub use extract::{Extractor, ExtractorError, SkipReason};
pub use index::{MAX_FILE_BYTES, SkippedFile, SymbolIndex, index_tree};
