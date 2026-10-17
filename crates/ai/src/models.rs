//! The model IDs the app uses. They're pinned here so they change in one place.

/// The model a new chat uses unless the user picks another.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

/// The models offered in the picker.
pub const MODELS: &[&str] = &["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"];
