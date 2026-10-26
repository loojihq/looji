//! How times are shown and typed: a 24-hour or 12-hour clock, and a fixed offset from UTC. The offset is a fixed
//! number of minutes for now, so it doesn't follow daylight saving. Named zones come later through `zone`.

use std::fmt;

/// Whether times read as 09:00 or 9:00 AM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clock {
    #[default]
    TwentyFour,
    Twelve,
}

impl Clock {
    pub const ALL: [Clock; 2] = [Clock::TwentyFour, Clock::Twelve];

    pub fn stored(self) -> &'static str {
        match self {
            Clock::TwentyFour => "24h",
            Clock::Twelve => "12h",
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|clock| clock.stored() == text)
    }

    pub fn label(self) -> &'static str {
        match self {
            Clock::TwentyFour => "24-hour",
            Clock::Twelve => "12-hour (AM/PM)",
        }
    }
}

/// A fixed offset from UTC, in whole minutes. Positive is east of Greenwich: +180 is UTC+03:00.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UtcOffset(i32);

impl UtcOffset {
    pub const UTC: UtcOffset = UtcOffset(0);

    /// The offset in minutes, or `None` when it's outside the range a real zone uses (-12:00 to +14:00).
    pub fn from_minutes(minutes: i32) -> Option<Self> {
        (-720..=840)
            .contains(&minutes)
            .then_some(UtcOffset(minutes))
    }

    pub fn minutes(self) -> i32 {
        self.0
    }

    /// The offset as the setting stores it: `+03:00`, `-05:30`, or `UTC` for zero.
    pub fn stored(self) -> String {
        if self.0 == 0 {
            return "UTC".to_string();
        }
        let sign = if self.0 < 0 { '-' } else { '+' };
        let absolute = self.0.abs();
        format!("{sign}{:02}:{:02}", absolute / 60, absolute % 60)
    }

    /// The offset a stored `+03:00`, `-05:30`, or `UTC` stands for.
    pub fn from_stored(text: &str) -> Option<Self> {
        if text == "UTC" {
            return Some(UtcOffset::UTC);
        }
        let sign = match text.chars().next()? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        let (hours, minutes) = text[1..].split_once(':')?;
        let hours: i32 = hours.parse().ok()?;
        let minutes: i32 = minutes.parse().ok()?;
        if minutes >= 60 {
            return None;
        }
        Self::from_minutes(sign * (hours * 60 + minutes))
    }
}

impl fmt::Display for UtcOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            write!(f, "UTC")
        } else {
            write!(f, "UTC{}", self.stored())
        }
    }
}

/// A time of day as the clock shows it, such as `09:00` or `9:00 AM`. Minute 1440 is midnight at the end of the
/// day, and reads as `24:00` or `12:00 AM` of the next day, as the working schedule shows it.
pub fn format_clock_time(minutes: u16, clock: Clock) -> String {
    match clock {
        Clock::TwentyFour => format!("{:02}:{:02}", minutes / 60, minutes % 60),
        Clock::Twelve => {
            let hour24 = (minutes / 60) % 24;
            let suffix = if hour24 < 12 { "AM" } else { "PM" };
            let hour12 = match hour24 % 12 {
                0 => 12,
                hour => hour,
            };
            format!("{hour12}:{:02} {suffix}", minutes % 60)
        }
    }
}

/// Reads a time typed on either clock: `09:00`, `9:00`, `9:00 am`, or `9:00pm`. Returns the minute of the day, with
/// `24:00` as 1440 and `12:00 AM` as midnight at the start of the day.
pub fn parse_clock_time(text: &str) -> Option<u16> {
    let text = text.trim().to_ascii_lowercase();
    let (time, meridiem) = match text.strip_suffix("am") {
        Some(rest) => (rest.trim_end(), Some(false)),
        None => match text.strip_suffix("pm") {
            Some(rest) => (rest.trim_end(), Some(true)),
            None => (text.as_str(), None),
        },
    };
    let (hours, minutes) = time.split_once(':')?;
    if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    if minutes >= 60 {
        return None;
    }
    let minute_of_day = match meridiem {
        None if hours == 24 && minutes == 0 => return Some(1440),
        None if hours < 24 => hours * 60 + minutes,
        None => return None,
        Some(pm) if (1..=12).contains(&hours) => {
            let hour24 = match (hours % 12, pm) {
                (hour, true) => hour + 12,
                (hour, false) => hour,
            };
            hour24 * 60 + minutes
        }
        Some(_) => return None,
    };
    Some(minute_of_day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_round_trips_through_its_stored_form() {
        for minutes in [0, 180, -330, 345, -720, 840] {
            let offset = UtcOffset::from_minutes(minutes).unwrap();
            assert_eq!(UtcOffset::from_stored(&offset.stored()), Some(offset));
        }
        assert_eq!(UtcOffset::from_minutes(900), None);
        assert_eq!(UtcOffset::from_stored("+03:75"), None);
        assert_eq!(UtcOffset::from_stored("Europe/London"), None);
    }

    #[test]
    fn the_twelve_hour_clock_reads_and_writes_am_and_pm() {
        assert_eq!(format_clock_time(540, Clock::Twelve), "9:00 AM");
        assert_eq!(format_clock_time(1020, Clock::Twelve), "5:00 PM");
        assert_eq!(format_clock_time(0, Clock::Twelve), "12:00 AM");
        assert_eq!(format_clock_time(720, Clock::Twelve), "12:00 PM");
        assert_eq!(parse_clock_time("9:00 am"), Some(540));
        assert_eq!(parse_clock_time("5:30pm"), Some(1050));
        assert_eq!(parse_clock_time("12:00 AM"), Some(0));
        assert_eq!(parse_clock_time("12:00 PM"), Some(720));
        assert_eq!(parse_clock_time("13:00 pm"), None);
    }

    #[test]
    fn the_twenty_four_hour_clock_still_reads_plain_times() {
        assert_eq!(format_clock_time(540, Clock::TwentyFour), "09:00");
        assert_eq!(parse_clock_time("17:30"), Some(1050));
        assert_eq!(parse_clock_time("24:00"), Some(1440));
        assert_eq!(parse_clock_time("9:5"), None);
    }
}
