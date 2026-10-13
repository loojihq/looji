use crate::error::DomainError;

/// Manual ordering key for tasks and columns.
///
/// It's text so keys can be inserted between two neighbours later (fractional
/// indexing) without renumbering. Ordering is lexicographic, so the key
/// alphabet must sort the way the ordering scheme expects.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(String);

impl Position {
    pub fn new(key: impl Into<String>) -> Result<Self, DomainError> {
        let key = key.into();
        if key.is_empty() {
            return Err(DomainError::EmptyPosition);
        }
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
