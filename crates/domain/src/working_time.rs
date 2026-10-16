//! Working time: which days are working days, and how many working minutes a day holds. This is the one
//! place that answers "is this a working day", so the workload view and the working-time due dates
//! (when they exist) can't disagree.
//!
//! The calendar is a stand-in for now. Per-project working hours don't exist yet, so `stub` gives every
//! project Monday to Friday at eight hours a day. Only `project_calendar` in the app uses it, so it's
//! replaced in one place when real settings land.

use crate::task_fields::DueDate;

/// Monday is 0 and Sunday is 6. 1970-01-01 was a Thursday, which is 3.
pub fn weekday(day: DueDate) -> usize {
    (day.day_number() + 3).rem_euclid(7) as usize
}

/// Which weekdays work, and how many minutes a working day holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkingCalendar {
    /// Indexed Monday to Sunday.
    pub workdays: [bool; 7],
    pub minutes_per_day: u32,
}

impl WorkingCalendar {
    /// Monday to Friday, eight hours a day. A stand-in, not a project's real hours.
    pub fn stub() -> Self {
        Self {
            workdays: [true, true, true, true, true, false, false],
            minutes_per_day: 8 * 60,
        }
    }

    /// Whether a day is a working day.
    pub fn is_working_day(&self, day: DueDate) -> bool {
        self.workdays[weekday(day)]
    }
}

/// A period of calendar days. Both ends are included, so "Monday to Friday" is five days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub first: DueDate,
    pub last: DueDate,
}

impl Period {
    /// `None` when the last day is before the first.
    pub fn new(first: DueDate, last: DueDate) -> Option<Self> {
        (last >= first).then_some(Self { first, last })
    }

    /// The Monday to Sunday week that contains `day`.
    pub fn week_containing(day: DueDate) -> Self {
        let monday = day.plus_days(-(weekday(day) as i64));
        Self {
            first: monday,
            last: monday.plus_days(6),
        }
    }

    /// How many days the period has, counting both ends.
    pub fn days(&self) -> i64 {
        self.last.day_number() - self.first.day_number() + 1
    }

    pub fn contains(&self, day: DueDate) -> bool {
        self.first <= day && day <= self.last
    }

    /// The same number of days, moved forwards or backwards.
    pub fn shifted(&self, days: i64) -> Self {
        Self {
            first: self.first.plus_days(days),
            last: self.last.plus_days(days),
        }
    }
}

/// How many working days a period holds under a calendar.
pub fn working_days(calendar: &WorkingCalendar, period: Period) -> i64 {
    (0..period.days())
        .map(|offset| period.first.plus_days(offset))
        .filter(|day| calendar.is_working_day(*day))
        .count() as i64
}

/// The working minutes a period holds: its working days times the minutes in one.
pub fn available_minutes(calendar: &WorkingCalendar, period: Period) -> i64 {
    working_days(calendar, period) * i64::from(calendar.minutes_per_day)
}

/// Minutes as hours for display: `32h`, or `32.5h` when there's a half hour. Rounded to the nearest
/// quarter hour for display only, since the stored value stays in whole minutes.
pub fn format_hours(minutes: i64) -> String {
    let quarters = (minutes as f64 / 15.0).round() as i64;
    let hours = quarters / 4;
    match quarters % 4 {
        0 => format!("{hours}h"),
        1 => format!("{hours}.25h"),
        2 => format!("{hours}.5h"),
        _ => format!("{hours}.75h"),
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        task_fields::DueDate,
        working_time::{
            Period, WorkingCalendar, available_minutes, format_hours, weekday, working_days,
        },
    };

    fn day(year: i32, month: u32, day: u32) -> DueDate {
        DueDate::new(year, month, day).unwrap()
    }

    #[test]
    fn monday_is_zero_and_the_week_starts_there() {
        assert_eq!(weekday(day(2026, 10, 12)), 0, "12 October 2026 is a Monday");
        assert_eq!(weekday(day(2026, 10, 18)), 6, "18 October 2026 is a Sunday");
        assert_eq!(weekday(day(2000, 1, 1)), 5, "1 January 2000 was a Saturday");
    }

    #[test]
    fn the_stub_works_monday_to_friday() {
        let calendar = WorkingCalendar::stub();
        assert!(calendar.is_working_day(day(2026, 10, 16)), "a Friday");
        assert!(!calendar.is_working_day(day(2026, 10, 17)), "a Saturday");
        assert!(!calendar.is_working_day(day(2026, 10, 18)), "a Sunday");
    }

    #[test]
    fn the_end_day_of_a_period_counts() {
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        assert_eq!(period.days(), 5);
        assert_eq!(working_days(&WorkingCalendar::stub(), period), 5);
    }

    #[test]
    fn a_period_that_ends_before_it_starts_is_refused() {
        assert!(Period::new(day(2026, 10, 16), day(2026, 10, 12)).is_none());
    }

    #[test]
    fn a_weekend_period_has_no_working_days() {
        let period = Period::new(day(2026, 10, 17), day(2026, 10, 18)).unwrap();
        assert_eq!(working_days(&WorkingCalendar::stub(), period), 0);
        assert_eq!(available_minutes(&WorkingCalendar::stub(), period), 0);
    }

    #[test]
    fn a_week_containing_a_wednesday_runs_monday_to_sunday() {
        let week = Period::week_containing(day(2026, 10, 14));
        assert_eq!(week.first, day(2026, 10, 12));
        assert_eq!(week.last, day(2026, 10, 18));
    }

    #[test]
    fn a_period_across_a_month_and_a_leap_day_counts_correctly() {
        // 26 February to 3 March 2028: 2028 is a leap year, so February has 29 days.
        let period = Period::new(day(2028, 2, 26), day(2028, 3, 3)).unwrap();
        assert_eq!(period.days(), 7);
        // 26 February 2028 is a Saturday, so the working days are 28 and 29 February, then 1, 2 and 3 March.
        assert_eq!(working_days(&WorkingCalendar::stub(), period), 5);
    }

    #[test]
    fn shifting_keeps_the_length() {
        let period = Period::new(day(2026, 10, 12), day(2026, 10, 16)).unwrap();
        let next = period.shifted(7);
        assert_eq!(next.first, day(2026, 10, 19));
        assert_eq!(next.days(), period.days());
    }

    #[test]
    fn hours_print_to_the_quarter_hour() {
        assert_eq!(format_hours(32 * 60), "32h");
        assert_eq!(format_hours(32 * 60 + 30), "32.5h");
        assert_eq!(format_hours(90), "1.5h");
        assert_eq!(format_hours(0), "0h");
    }
}
