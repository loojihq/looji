//! The rules for the fields of a task that the create and edit dialog edits: priority, estimate,
//! due date, and tags. Each rule is a pure function, so it's tested without a window or a database.
//!
//! Estimates are stored in whole minutes. Blank means no estimate, and zero is a real estimate of
//! zero hours. Due dates are plain calendar days, with no time and no zone.

use std::fmt;
use std::str::FromStr;

use unicode_normalization::UnicodeNormalization;

use crate::{error::DomainError, task::Estimate, validation::MAX_TEXT_CHARS};

/// How many tags one task can have.
pub const MAX_TAGS: usize = 20;
/// The longest single tag, in characters.
pub const MAX_TAG_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Priority::Low, Priority::Medium, Priority::High];

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
        }
    }
}

impl FromStr for Priority {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            other => Err(DomainError::UnknownPriority(other.to_string())),
        }
    }
}

/// Why an estimate was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstimateError {
    NotANumber,
    Negative,
    TooLarge,
}

impl fmt::Display for EstimateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EstimateError::NotANumber => write!(f, "Enter hours as a number, like 2 or 2.5."),
            EstimateError::Negative => write!(f, "An estimate can't be negative."),
            EstimateError::TooLarge => write!(f, "That estimate is too large."),
        }
    }
}

/// Reads an estimate in hours. Blank is no estimate. Both `.` and `,` work as the decimal mark,
/// so `2,5` and `2.5` mean the same thing. Zero is accepted. Infinite and NaN values are rejected.
pub fn parse_estimate_hours(text: &str) -> Result<Option<Estimate>, EstimateError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let normalized = if text.contains('.') {
        text.to_string()
    } else {
        text.replace(',', ".")
    };
    if normalized.contains(',') {
        return Err(EstimateError::NotANumber);
    }
    let hours: f64 = normalized.parse().map_err(|_| EstimateError::NotANumber)?;
    if !hours.is_finite() {
        return Err(EstimateError::NotANumber);
    }
    if hours < 0.0 {
        return Err(EstimateError::Negative);
    }
    Estimate::from_hours(hours)
        .map(Some)
        .map_err(|_| EstimateError::TooLarge)
}

/// A calendar day, with no time and no zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DueDate {
    year: i32,
    month: u32,
    day: u32,
}

impl DueDate {
    /// `None` if the day doesn't exist, including February 29 in a common year.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u32 {
        self.month
    }

    pub fn day(self) -> u32 {
        self.day
    }
}

impl fmt::Display for DueDate {
    /// `YYYY-MM-DD`, which is also the stored form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl FromStr for DueDate {
    type Err = DomainError;

    /// Accepts only `YYYY-MM-DD` with a real day.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || DomainError::InvalidDueDate(text.to_string());
        let bytes = text.as_bytes();
        let shaped = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
        if !shaped {
            return Err(invalid());
        }
        let year = text[0..4].parse().map_err(|_| invalid())?;
        let month = text[5..7].parse().map_err(|_| invalid())?;
        let day = text[8..10].parse().map_err(|_| invalid())?;
        DueDate::new(year, month, day).ok_or_else(invalid)
    }
}

/// Reads a due date, where blank means no due date.
pub fn parse_due_date(text: &str) -> Result<Option<DueDate>, DomainError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse().map(Some)
}

/// Why a tag was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    Empty,
    TooLong { max: usize },
    ContainsComma,
    TooMany { max: usize },
}

impl fmt::Display for TagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagError::Empty => write!(f, "A tag can't be empty."),
            TagError::TooLong { max } => write!(f, "A tag can be at most {max} characters."),
            TagError::ContainsComma => write!(f, "A tag can't contain a comma."),
            TagError::TooMany { max } => write!(f, "A task can have at most {max} tags."),
        }
    }
}

/// Checks one tag and returns it in stored form. Every entry point uses this one rule, so a tag typed
/// in the dialog, on a card, or in the panel is stored the same way. Runs of whitespace become one
/// space. The text is put in composed Unicode form, so an accented letter typed as one character
/// and as a letter plus an accent are the same tag. Then it's lowercased.
pub fn normalize_tag(text: &str) -> Result<String, TagError> {
    let spaced = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let tag: String = spaced
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect();
    if tag.is_empty() {
        return Err(TagError::Empty);
    }
    if tag.contains(',') {
        return Err(TagError::ContainsComma);
    }
    if tag.chars().count() > MAX_TAG_CHARS {
        return Err(TagError::TooLong { max: MAX_TAG_CHARS });
    }
    Ok(tag)
}

/// Normalizes a set of tags: each one checked, duplicates dropped, sorted. Sorting means the order
/// tags were typed in doesn't matter, so reordering them is not a change.
pub fn normalize_tags<'a>(
    tags: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, TagError> {
    let mut stored: Vec<String> = Vec::new();
    for tag in tags {
        let tag = normalize_tag(tag)?;
        if !stored.contains(&tag) {
            stored.push(tag);
        }
    }
    stored.sort();
    if stored.len() > MAX_TAGS {
        return Err(TagError::TooMany { max: MAX_TAGS });
    }
    Ok(stored)
}

/// Checks a title for the dialog: trimmed, not empty, and within the length limit.
pub fn validate_title(title: &str) -> Result<String, DomainError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(DomainError::EmptyTitle);
    }
    if trimmed.chars().count() > MAX_TEXT_CHARS {
        return Err(DomainError::TooLong {
            max: MAX_TEXT_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{
        DueDate, EstimateError, MAX_TAG_CHARS, MAX_TAGS, Priority, TagError, normalize_tag,
        normalize_tags, parse_due_date, parse_estimate_hours, validate_title,
    };
    use crate::error::DomainError;

    fn minutes(text: &str) -> Option<u32> {
        parse_estimate_hours(text).unwrap().map(|e| e.minutes())
    }

    #[test]
    fn a_blank_estimate_means_no_estimate() {
        assert_eq!(minutes(""), None);
        assert_eq!(minutes("   "), None);
    }

    #[test]
    fn negative_estimates_are_rejected() {
        assert_eq!(parse_estimate_hours("-1"), Err(EstimateError::Negative));
        assert_eq!(parse_estimate_hours("-0.5"), Err(EstimateError::Negative));
    }

    #[test]
    fn zero_is_a_real_estimate() {
        assert_eq!(minutes("0"), Some(0));
    }

    #[test]
    fn decimals_are_read_in_minutes() {
        assert_eq!(minutes("2.5"), Some(150));
        assert_eq!(minutes("2"), Some(120));
    }

    #[test]
    fn a_decimal_comma_means_the_same_as_a_point() {
        assert_eq!(minutes("2,5"), Some(150));
        assert_eq!(
            parse_estimate_hours("2,5,1"),
            Err(EstimateError::NotANumber)
        );
    }

    #[test]
    fn text_is_not_a_number() {
        assert_eq!(parse_estimate_hours("abc"), Err(EstimateError::NotANumber));
        assert_eq!(parse_estimate_hours("2h"), Err(EstimateError::NotANumber));
    }

    #[test]
    fn infinite_and_nan_are_rejected() {
        assert_eq!(parse_estimate_hours("inf"), Err(EstimateError::NotANumber));
        assert_eq!(parse_estimate_hours("NaN"), Err(EstimateError::NotANumber));
    }

    #[test]
    fn an_empty_title_is_rejected_and_a_real_one_is_trimmed() {
        assert_eq!(validate_title("   "), Err(DomainError::EmptyTitle));
        assert_eq!(validate_title("  Ship  ").unwrap(), "Ship");
        assert!(validate_title(&"x".repeat(201)).is_err());
    }

    #[test]
    fn due_dates_must_be_real_calendar_days() {
        assert_eq!(parse_due_date("").unwrap(), None);
        assert_eq!(
            parse_due_date("2026-10-14").unwrap(),
            DueDate::new(2026, 10, 14)
        );
        assert!(
            parse_due_date("2026-02-29").is_err(),
            "2026 is not a leap year"
        );
        assert!(parse_due_date("2028-02-29").is_ok());
        assert!(parse_due_date("14/10/2026").is_err());
        assert!(parse_due_date("2026-13-01").is_err());
    }

    #[test]
    fn a_due_date_prints_in_its_stored_form() {
        assert_eq!(DueDate::new(2026, 1, 5).unwrap().to_string(), "2026-01-05");
    }

    #[test]
    fn priorities_round_trip_through_their_stored_form() {
        for priority in Priority::ALL {
            assert_eq!(priority.as_str().parse::<Priority>().unwrap(), priority);
        }
        assert!("urgent".parse::<Priority>().is_err());
    }

    #[test]
    fn tags_are_trimmed_lowercased_and_checked() {
        assert_eq!(normalize_tag("  Design ").unwrap(), "design");
        assert_eq!(normalize_tag(" Foo Bar ").unwrap(), "foo bar");
        assert_eq!(normalize_tag("  "), Err(TagError::Empty));
        assert_eq!(normalize_tag(""), Err(TagError::Empty));
        assert_eq!(normalize_tag("a,b"), Err(TagError::ContainsComma));
        assert_eq!(
            normalize_tag(&"x".repeat(MAX_TAG_CHARS + 1)),
            Err(TagError::TooLong { max: MAX_TAG_CHARS })
        );
    }

    #[test]
    fn tags_that_differ_only_in_case_or_spacing_are_the_same_tag() {
        assert_eq!(normalize_tag("FOO").unwrap(), normalize_tag("foo").unwrap());
        assert_eq!(
            normalize_tag("foo   bar").unwrap(),
            normalize_tag("Foo Bar").unwrap()
        );
    }

    #[test]
    fn composed_and_decomposed_accents_are_the_same_tag() {
        // "é" as one character, and as "e" followed by a combining acute accent.
        let composed = normalize_tag("caf\u{e9}").unwrap();
        let decomposed = normalize_tag("cafe\u{301}").unwrap();
        assert_eq!(composed, decomposed);
        assert_eq!(composed, "caf\u{e9}");
    }

    #[test]
    fn unicode_capitals_are_lowercased() {
        assert_eq!(normalize_tag("ÉCOLE").unwrap(), "école");
    }

    #[test]
    fn tag_sets_drop_duplicates_and_do_not_depend_on_order() {
        assert_eq!(
            normalize_tags(["web", "Design", "web"]).unwrap(),
            ["design", "web"]
        );
        assert_eq!(
            normalize_tags(["design", "web"]).unwrap(),
            normalize_tags(["web", "design"]).unwrap()
        );
    }

    #[test]
    fn a_task_cannot_have_too_many_tags() {
        let many: Vec<String> = (0..=MAX_TAGS).map(|n| format!("tag{n}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            normalize_tags(refs),
            Err(TagError::TooMany { max: MAX_TAGS })
        );
    }
}
