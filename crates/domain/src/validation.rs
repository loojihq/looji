use crate::error::DomainError;

/// Longest title or name, counted in characters after trimming.
pub const MAX_TEXT_CHARS: usize = 200;

/// Trims `value`, then checks that the result is non-empty and within [`MAX_TEXT_CHARS`].
/// `empty` is the error to return when nothing is left after trimming.
pub(crate) fn trimmed_text(value: &str, empty: DomainError) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(empty);
    }
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        return Err(DomainError::TooLong {
            max: MAX_TEXT_CHARS,
        });
    }
    Ok(trimmed.to_string())
}
