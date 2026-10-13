use std::time::{SystemTime, UNIX_EPOCH};

/// An instant in UTC, stored as milliseconds since the Unix epoch.
///
/// Local time and time zones belong to the UI, not the domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        Self(millis)
    }

    pub fn from_unix_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub fn unix_millis(&self) -> i64 {
        self.0
    }
}
