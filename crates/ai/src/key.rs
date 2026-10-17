//! The API key, wrapped so it can't reach a log or an error message by accident.

/// An API key. Its `Debug` output is `***`, and nothing else in the crate prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key. Blank text is refused, since it can't be a key.
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_string()))
    }

    /// The key itself, for the one request that needs it.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::ApiKey;

    #[test]
    fn debug_output_never_shows_the_key() {
        let key = ApiKey::new("sk-ant-secret-value").unwrap();
        assert_eq!(format!("{key:?}"), "***");
        assert!(!format!("{key:?}").contains("secret"));
    }

    #[test]
    fn blank_text_is_refused_as_a_key() {
        assert!(ApiKey::new("   ").is_none());
        assert!(ApiKey::new("").is_none());
    }
}
