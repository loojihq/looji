//! The working-time due date: the moment that a given amount of working time after a start has passed.
//!
//! The arithmetic is on whole minutes, so repeated subtraction over many weeks can't drift. The clock is an
//! input, never read here, so every result is the same for the same input.
//!
//! Boundary rules, written down:
//! - Working time is the schedule's window (start to end) on each workday. Nothing else counts.
//! - A start before the window's start is moved to the window's start on the same day, if that day works,
//!   and otherwise to the window's start on the next workday.
//! - A start at or after the window's end is outside working time. It moves to the next workday's start.
//!   So a start at 17:00 counts as the next morning, not as the end of the day.
//! - A task that finishes exactly at the window's end finishes at that end, on that day. It doesn't roll to
//!   the next morning.
//! - A zero estimate returns the start unchanged, even when the start is outside working time.
//! - A fractional estimate is converted to whole minutes once, by rounding, and then works through like any
//!   other. A quarter hour is 15 minutes.
//! - A negative or non-finite estimate is an error. The function never invents an estimate.
//!
//! The times are wall-clock with no zone, the same as the schedule and due dates. Daylight-saving changes
//! therefore can't arise here: a stored start is a day and a minute, and nothing converts it to an instant.
//! Working out how a local time that doesn't exist or happens twice on such a day should be read is the job
//! of whichever change first assigns a zone to a project. The date rules in this file would need to be
//! extended at that point, and the test table extended with them.

use crate::{
    error::DomainError, status::Status, task::Estimate, task_fields::DueDate, time::Timestamp,
    working_schedule::WorkingSchedule, working_time::weekday,
};

/// A wall-clock moment: a calendar day, and the minutes since midnight. 1440 is the end of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalMoment {
    pub day: DueDate,
    pub minute: u16,
}

impl LocalMoment {
    pub fn new(day: DueDate, minute: u16) -> Self {
        Self { day, minute }
    }

    /// The UTC wall-clock moment of a timestamp. The project zone isn't stored yet, so "now" is read in UTC,
    /// and this is the one place that decides that.
    pub fn from_timestamp_utc(at: Timestamp) -> Self {
        let millis = at.unix_millis();
        let day = DueDate::from_day_number(millis.div_euclid(86_400_000));
        let minute = (millis.rem_euclid(86_400_000) / 60_000) as u16;
        Self { day, minute }
    }

    /// The start of a day. Work from it begins at the schedule's start on that day, or on the next working day
    /// if that day doesn't work. This is how a task with only a start date is started.
    pub fn start_of(day: DueDate) -> Self {
        Self { day, minute: 0 }
    }
}

/// The first moment at or after `at` that is inside working time. The caller has checked that the window
/// is non-empty and that at least one weekday works.
fn enter_working_time(workdays: &[bool; 7], start: u16, end: u16, at: LocalMoment) -> LocalMoment {
    let mut at = at;
    loop {
        if workdays[weekday(at.day)] && at.minute < end {
            return LocalMoment {
                day: at.day,
                minute: at.minute.max(start),
            };
        }
        at = LocalMoment {
            day: at.day.plus_days(1),
            minute: 0,
        };
    }
}

/// The moment that `minutes` of working time after `start` has passed, on the days marked in `workdays`,
/// with each working day running from `start` to `end` in minutes since midnight.
///
/// An empty set of days, or a window with no length, is refused before anything is counted. Without that
/// check the search for the next working time would never end.
fn advance(
    workdays: [bool; 7],
    start: u16,
    end: u16,
    from: LocalMoment,
    minutes: u32,
) -> Result<LocalMoment, DomainError> {
    if !workdays.contains(&true) {
        return Err(DomainError::Schedule(
            crate::working_schedule::ScheduleError::NoWorkdays,
        ));
    }
    if end <= start {
        return Err(DomainError::Schedule(
            crate::working_schedule::ScheduleError::EmptyShift { minutes: start },
        ));
    }
    if minutes == 0 {
        return Ok(from);
    }
    let mut remaining = i64::from(minutes);
    let mut cursor = from;
    loop {
        cursor = enter_working_time(&workdays, start, end, cursor);
        let left = i64::from(end) - i64::from(cursor.minute);
        if remaining <= left {
            return Ok(LocalMoment {
                day: cursor.day,
                minute: u16::try_from(i64::from(cursor.minute) + remaining)
                    .expect("the result is within the window, so it fits in a day"),
            });
        }
        remaining -= left;
        cursor = LocalMoment::start_of(cursor.day.plus_days(1));
    }
}

/// The moment that `minutes` of working time after `start` has passed, under `schedule`.
pub fn add_working_minutes(
    schedule: &WorkingSchedule,
    start: LocalMoment,
    minutes: u32,
) -> Result<LocalMoment, DomainError> {
    let workdays = schedule.calendar().workdays;
    advance(workdays, schedule.start(), schedule.end(), start, minutes)
}

/// The moment that `hours` of working time after `start` has passed. Hours are rounded to whole minutes
/// once. A negative or non-finite estimate is refused.
pub fn add_working_hours(
    schedule: &WorkingSchedule,
    start: LocalMoment,
    hours: f64,
) -> Result<LocalMoment, DomainError> {
    if hours.is_finite() && hours < 0.0 {
        return Err(DomainError::NegativeEstimate {
            minutes: (hours * 60.0).round() as i64,
        });
    }
    let estimate = Estimate::from_hours(hours)?;
    add_working_minutes(schedule, start, estimate.minutes())
}

/// Whether moving a task from one status to another should set its due date. It should, when the task first
/// enters In progress, has no due date, and has an estimate of more than zero. A move within one column, or a
/// task that already has a date, changes nothing. A zero estimate counts as none.
pub fn schedules_on_move(
    from: Status,
    to: Status,
    due: Option<DueDate>,
    estimate_minutes: Option<u32>,
) -> bool {
    from != to
        && to == Status::InProgress
        && due.is_none()
        && estimate_minutes.is_some_and(|minutes| minutes > 0)
}

/// Whether a task is in progress without a due date or an estimate, so it should show a hint. Zero counts as
/// no estimate.
pub fn needs_estimate_hint(
    status: Status,
    due: Option<DueDate>,
    estimate_minutes: Option<u32>,
) -> bool {
    status == Status::InProgress
        && due.is_none()
        && estimate_minutes.is_none_or(|minutes| minutes == 0)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        due_calculation::{LocalMoment, add_working_hours, add_working_minutes},
        error::DomainError,
        task_fields::DueDate,
        working_schedule::{Workdays, WorkingSchedule},
    };

    fn day(year: i32, month: u32, day: u32) -> DueDate {
        DueDate::new(year, month, day).unwrap()
    }

    fn at(year: i32, month: u32, day_of_month: u32, hour: u16, minute: u16) -> LocalMoment {
        LocalMoment::new(day(year, month, day_of_month), hour * 60 + minute)
    }

    /// Monday to Friday, 09:00 to 17:00.
    fn standard() -> WorkingSchedule {
        WorkingSchedule::default()
    }

    /// Sunday to Thursday, 08:00 to 16:00.
    fn sunday_to_thursday() -> WorkingSchedule {
        WorkingSchedule::try_new(8 * 60, 16 * 60, Workdays::from_mask(0b100_1111).unwrap()).unwrap()
    }

    /// One case of the table: a name that's printed when it fails, the inputs, and the expected moment.
    struct Case {
        name: &'static str,
        schedule: WorkingSchedule,
        start: LocalMoment,
        hours: f64,
        expected: LocalMoment,
    }

    #[test]
    fn working_time_is_added_and_skips_non_working_time() {
        // 12 October 2026 is a Monday, 16 October a Friday, 17 and 18 October the weekend.
        let cases = vec![
            Case {
                name: "a weekday start inside hours stays on the same day",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 2.0,
                expected: at(2026, 10, 12, 11, 0),
            },
            Case {
                name: "Friday 16:00 plus two hours finishes on Monday at 10:00",
                schedule: standard(),
                start: at(2026, 10, 16, 16, 0),
                hours: 2.0,
                expected: at(2026, 10, 19, 10, 0),
            },
            Case {
                name: "a Saturday start counts from Monday 09:00",
                schedule: standard(),
                start: at(2026, 10, 17, 10, 0),
                hours: 2.0,
                expected: at(2026, 10, 19, 11, 0),
            },
            Case {
                name: "a start at exactly 17:00 is the next morning",
                schedule: standard(),
                start: at(2026, 10, 12, 17, 0),
                hours: 2.0,
                expected: at(2026, 10, 13, 11, 0),
            },
            Case {
                name: "a start before the day opens waits for it",
                schedule: standard(),
                start: at(2026, 10, 12, 6, 0),
                hours: 1.0,
                expected: at(2026, 10, 12, 10, 0),
            },
            Case {
                name: "an estimate that runs past the end of the day continues the next morning",
                schedule: standard(),
                start: at(2026, 10, 12, 16, 30),
                hours: 1.0,
                expected: at(2026, 10, 13, 9, 30),
            },
            Case {
                name: "finishing exactly at 17:00 stays on the same day",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 8.0,
                expected: at(2026, 10, 12, 17, 0),
            },
            Case {
                name: "forty hours from Monday 09:00 finishes Friday at 17:00, not the next Monday",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 40.0,
                expected: at(2026, 10, 16, 17, 0),
            },
            Case {
                name: "eighty hours is two working weeks, ending Friday 23 October at 17:00",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 80.0,
                expected: at(2026, 10, 23, 17, 0),
            },
            Case {
                name: "half an hour",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 0.5,
                expected: at(2026, 10, 12, 9, 30),
            },
            Case {
                name: "a quarter of an hour",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 0.25,
                expected: at(2026, 10, 12, 9, 15),
            },
            Case {
                name: "a zero estimate returns the start unchanged",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 0.0,
                expected: at(2026, 10, 12, 9, 0),
            },
            Case {
                name: "a zero estimate from a weekend keeps the weekend start",
                schedule: standard(),
                start: at(2026, 10, 17, 10, 0),
                hours: 0.0,
                expected: at(2026, 10, 17, 10, 0),
            },
            Case {
                name: "a custom Sunday-to-Thursday week skips Friday and Saturday",
                schedule: sunday_to_thursday(),
                start: at(2026, 10, 15, 15, 0),
                hours: 2.0,
                expected: at(2026, 10, 18, 9, 0),
            },
            Case {
                name: "a custom week starting on the Sunday",
                schedule: sunday_to_thursday(),
                start: at(2026, 10, 18, 8, 0),
                hours: 8.0,
                expected: at(2026, 10, 18, 16, 0),
            },
            Case {
                name: "the day is a whole number of minutes, so eighty minutes are exact",
                schedule: standard(),
                start: at(2026, 10, 12, 9, 0),
                hours: 1.0 + 1.0 / 3.0,
                expected: at(2026, 10, 12, 10, 20),
            },
        ];

        for case in cases {
            let result = add_working_hours(&case.schedule, case.start, case.hours)
                .unwrap_or_else(|err| panic!("{}: {err}", case.name));
            assert_eq!(result, case.expected, "{}", case.name);
        }
    }

    #[test]
    fn a_start_with_only_a_date_begins_at_the_start_of_that_working_day() {
        // Friday 16 October from its start, eight hours: the whole Friday, finishing at 17:00.
        let result =
            add_working_hours(&standard(), LocalMoment::start_of(day(2026, 10, 16)), 8.0).unwrap();
        assert_eq!(result, at(2026, 10, 16, 17, 0));
    }

    #[test]
    fn a_start_with_only_a_date_on_a_weekend_begins_on_the_next_working_day() {
        let result =
            add_working_hours(&standard(), LocalMoment::start_of(day(2026, 10, 17)), 1.0).unwrap();
        assert_eq!(result, at(2026, 10, 19, 10, 0));
    }

    #[test]
    fn a_negative_or_unreadable_estimate_is_an_error_not_a_default() {
        let start = at(2026, 10, 12, 9, 0);
        assert_eq!(
            add_working_hours(&standard(), start, -1.0),
            Err(DomainError::NegativeEstimate { minutes: -60 })
        );
        assert_eq!(
            add_working_hours(&standard(), start, -0.001),
            Err(DomainError::NegativeEstimate { minutes: 0 }),
            "a tiny negative is still negative"
        );
        assert_eq!(
            add_working_hours(&standard(), start, f64::NAN),
            Err(DomainError::NotFinite)
        );
        assert_eq!(
            add_working_hours(&standard(), start, f64::INFINITY),
            Err(DomainError::NotFinite)
        );
    }

    #[test]
    fn the_result_is_always_inside_working_time_or_at_its_end() {
        // A sweep: from every quarter hour of a week, with estimates of every quarter hour, the result is never
        // on a day that doesn't work, and never after the end of the window.
        let schedule = standard();
        let start_day = day(2026, 10, 12);
        for start_minute in (0..1440).step_by(15) {
            for minutes in (0..=(8 * 60 * 3)).step_by(15) {
                let start = LocalMoment::new(start_day, start_minute);
                let result = add_working_minutes(&schedule, start, minutes).unwrap();
                assert!(
                    schedule
                        .workdays()
                        .contains(crate::working_time::weekday(result.day))
                        || minutes == 0,
                    "{start:?} + {minutes} min landed on a non-working day: {result:?}"
                );
                assert!(
                    result.minute <= schedule.end() || minutes == 0,
                    "{result:?}"
                );
                assert!(result >= start || minutes == 0, "time only moves forward");
            }
        }
    }

    #[test]
    fn an_empty_set_of_workdays_is_an_error_when_the_calculator_is_called_directly() {
        // A schedule can't hold no workdays, so the calculator is called with the raw days. It must refuse,
        // not search forever for a working day that never comes.
        let start = LocalMoment::start_of(day(2026, 10, 12));
        assert_eq!(
            super::advance([false; 7], 540, 1020, start, 60),
            Err(DomainError::Schedule(
                crate::working_schedule::ScheduleError::NoWorkdays
            ))
        );
    }

    #[test]
    fn an_empty_window_is_an_error_not_an_endless_search() {
        let start = LocalMoment::start_of(day(2026, 10, 12));
        assert_eq!(
            super::advance([true; 7], 540, 540, start, 60),
            Err(DomainError::Schedule(
                crate::working_schedule::ScheduleError::EmptyShift { minutes: 540 }
            ))
        );
    }
}
