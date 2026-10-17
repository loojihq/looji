//! A project's working schedule: the times its working day runs between, and the weekdays it runs on.
//!
//! Decisions this module enforces, and that the calculation and calendar tasks rely on:
//! - Times are wall-clock time with no time zone, like due dates. The app doesn't store a zone. How a
//!   local time that doesn't exist or happens twice on a daylight-saving change is read belongs to the
//!   due-date calculation, not to this type.
//! - Times are minutes since midnight, from 0 to 1440. 1440 is the end of the day, shown as "24:00".
//! - A shift must end after it starts on the same day. An overnight shift, such as 22:00 to 06:00, is
//!   refused for now, with a message that says so. It would wrap past midnight and complicate due-date
//!   arithmetic, and storing minutes means it can be supported later without a migration.
//! - The start and the end can't be the same time. That's zero hours, not "all day", so it's refused.
//! - At least one weekday must be a working day. Without one the due-date calculation would never
//!   find a working day.
//! - Weekdays are a 7-bit mask with Monday as bit 0 and Sunday as bit 6. A mask with any other bit set
//!   is refused, rather than keeping bits that mean nothing.
//!
//! Every value is checked when it's built, so an invalid schedule can't exist in memory. The database
//! repeats the time checks as CHECK constraints, and the defaults below are the same as the SQL defaults
//! in the migration that adds the columns. A test keeps the two in step.

use std::fmt;

use crate::working_time::WorkingCalendar;

/// The end of the day, in minutes since midnight. A shift may end here, and no later.
pub const MINUTES_PER_DAY: u16 = 1440;
/// 09:00.
pub const DEFAULT_START_MINUTES: u16 = 9 * 60;
/// 17:00.
pub const DEFAULT_END_MINUTES: u16 = 17 * 60;
/// Monday to Friday, as a mask: bits 0 to 4.
pub const MON_TO_FRI_MASK: u8 = 0b0001_1111;
/// Every bit a weekday mask may use.
const VALID_MASK_BITS: u8 = 0b0111_1111;

/// Why a schedule was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// The start and the end are the same time.
    EmptyShift { minutes: u16 },
    /// The end is before the start. This is an overnight shift, which isn't supported yet.
    Overnight { start: u16, end: u16 },
    /// A time is past the end of the day.
    TimeOutOfRange { minutes: i64 },
    /// No weekday is a working day.
    NoWorkdays,
    /// The weekday mask has a bit that doesn't name a day.
    UnknownWorkdayBits { mask: i64 },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::EmptyShift { minutes } => write!(
                f,
                "the working day can't start and end at the same time ({}). A working day needs a start before its end.",
                format_time(*minutes)
            ),
            ScheduleError::Overnight { start, end } => write!(
                f,
                "a working day that ends at {} is before it starts at {}. Overnight shifts are not supported yet.",
                format_time(*end),
                format_time(*start)
            ),
            ScheduleError::TimeOutOfRange { minutes } => write!(
                f,
                "{minutes} minutes is not a time of day. Times run from 00:00 to 24:00."
            ),
            ScheduleError::NoWorkdays => {
                write!(
                    f,
                    "choose at least one working day, or the project would have no working time"
                )
            }
            ScheduleError::UnknownWorkdayBits { mask } => {
                write!(
                    f,
                    "the workday mask {mask:#b} has a bit that is not a weekday"
                )
            }
        }
    }
}

/// Minutes since midnight as `HH:MM`. 1440 prints as "24:00", not "00:00", so it still sorts after the start.
pub fn format_time(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The weekdays a project works on. Monday is bit 0, through Sunday as bit 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Workdays(u8);

impl Workdays {
    /// Monday to Friday.
    pub const MON_TO_FRI: Workdays = Workdays(MON_TO_FRI_MASK);

    /// Checks a mask: at least one day, and no bit above Sunday.
    pub fn from_mask(mask: u8) -> Result<Self, ScheduleError> {
        if mask & !VALID_MASK_BITS != 0 {
            return Err(ScheduleError::UnknownWorkdayBits {
                mask: i64::from(mask),
            });
        }
        if mask == 0 {
            return Err(ScheduleError::NoWorkdays);
        }
        Ok(Self(mask))
    }

    pub fn mask(self) -> u8 {
        self.0
    }

    /// Whether the weekday with this index (Monday 0 to Sunday 6) is a working day.
    pub fn contains(self, weekday: usize) -> bool {
        weekday < 7 && self.0 & (1 << weekday) != 0
    }
}

/// A project's working day: when it starts and ends, and which weekdays it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkingSchedule {
    start: u16,
    end: u16,
    workdays: Workdays,
}

impl WorkingSchedule {
    /// Checks a schedule. Every constructor goes through here.
    pub fn try_new(start: u16, end: u16, workdays: Workdays) -> Result<Self, ScheduleError> {
        for minutes in [start, end] {
            if minutes > MINUTES_PER_DAY {
                return Err(ScheduleError::TimeOutOfRange {
                    minutes: i64::from(minutes),
                });
            }
        }
        if start == end {
            return Err(ScheduleError::EmptyShift { minutes: start });
        }
        if end < start {
            return Err(ScheduleError::Overnight { start, end });
        }
        Ok(Self {
            start,
            end,
            workdays,
        })
    }

    /// Reads stored values back, checking them as `try_new` does. The times arrive as `i64` because that is
    /// how the database holds them, so anything outside `u16` is refused rather than wrapped.
    pub fn from_stored(start: i64, end: i64, mask: i64) -> Result<Self, ScheduleError> {
        let time = |minutes: i64| {
            u16::try_from(minutes).map_err(|_| ScheduleError::TimeOutOfRange { minutes })
        };
        let mask_bits =
            u8::try_from(mask).map_err(|_| ScheduleError::UnknownWorkdayBits { mask })?;
        Self::try_new(time(start)?, time(end)?, Workdays::from_mask(mask_bits)?)
    }

    pub fn start(&self) -> u16 {
        self.start
    }

    pub fn end(&self) -> u16 {
        self.end
    }

    pub fn workdays(&self) -> Workdays {
        self.workdays
    }

    /// The working minutes in one working day.
    pub fn minutes_per_day(&self) -> u16 {
        self.end - self.start
    }

    /// The calendar the working-time calculations use: which days work, and how long each day is.
    pub fn calendar(&self) -> WorkingCalendar {
        let mut workdays = [false; 7];
        for (index, day) in workdays.iter_mut().enumerate() {
            *day = self.workdays.contains(index);
        }
        WorkingCalendar {
            workdays,
            minutes_per_day: u32::from(self.minutes_per_day()),
        }
    }
}

impl Default for WorkingSchedule {
    /// Monday to Friday, 09:00 to 17:00. New projects start with this.
    fn default() -> Self {
        Self {
            start: DEFAULT_START_MINUTES,
            end: DEFAULT_END_MINUTES,
            workdays: Workdays::MON_TO_FRI,
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::working_schedule::{
        DEFAULT_END_MINUTES, DEFAULT_START_MINUTES, ScheduleError, Workdays, WorkingSchedule,
        format_time,
    };

    #[test]
    fn the_default_is_monday_to_friday_nine_to_five() {
        let schedule = WorkingSchedule::default();
        assert_eq!(schedule.start(), 540);
        assert_eq!(schedule.end(), 1020);
        assert_eq!(schedule.workdays().mask(), 31);
        assert_eq!(schedule.minutes_per_day(), 480);
        assert_eq!((DEFAULT_START_MINUTES, DEFAULT_END_MINUTES), (540, 1020));
    }

    #[test]
    fn every_invalid_schedule_is_refused_with_its_reason() {
        let mon_fri = Workdays::MON_TO_FRI;
        let cases: Vec<(u16, u16, Result<(), ScheduleError>)> = vec![
            (540, 1020, Ok(())),
            (0, 1440, Ok(())),
            (1439, 1440, Ok(())),
            (540, 540, Err(ScheduleError::EmptyShift { minutes: 540 })),
            (
                1020,
                540,
                Err(ScheduleError::Overnight {
                    start: 1020,
                    end: 540,
                }),
            ),
            (
                0,
                1441,
                Err(ScheduleError::TimeOutOfRange { minutes: 1441 }),
            ),
        ];
        for (start, end, expected) in cases {
            let built = WorkingSchedule::try_new(start, end, mon_fri).map(|_| ());
            assert_eq!(built, expected, "start {start}, end {end}");
        }
    }

    #[test]
    fn an_overnight_shift_is_refused_with_a_message_that_says_so() {
        let error = WorkingSchedule::try_new(22 * 60, 6 * 60, Workdays::MON_TO_FRI).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Overnight shifts are not supported yet"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_day_set_is_refused() {
        assert_eq!(Workdays::from_mask(0), Err(ScheduleError::NoWorkdays));
    }

    #[test]
    fn a_mask_with_a_bit_above_sunday_is_refused_not_kept() {
        assert_eq!(
            Workdays::from_mask(0b1000_0001),
            Err(ScheduleError::UnknownWorkdayBits { mask: 0b1000_0001 })
        );
        assert!(
            Workdays::from_mask(0b0111_1111).is_ok(),
            "all seven days are fine"
        );
    }

    #[test]
    fn stored_values_are_checked_as_they_are_read() {
        assert_eq!(
            WorkingSchedule::from_stored(540, 1020, 31),
            Ok(WorkingSchedule::default())
        );
        assert!(
            WorkingSchedule::from_stored(1020, 540, 31).is_err(),
            "end before start"
        );
        assert!(
            WorkingSchedule::from_stored(-5, 1020, 31).is_err(),
            "negative time"
        );
        assert!(
            WorkingSchedule::from_stored(540, 1020, 0).is_err(),
            "no days"
        );
        assert!(
            WorkingSchedule::from_stored(540, 1020, 256).is_err(),
            "mask too wide"
        );
    }

    #[test]
    fn the_end_of_the_day_prints_as_midnight_the_next_day_not_as_zero() {
        assert_eq!(format_time(1440), "24:00");
        assert_eq!(format_time(0), "00:00");
        assert_eq!(format_time(540), "09:00");
        assert!(
            format_time(1440) > format_time(540),
            "24:00 still sorts after 09:00"
        );
    }

    #[test]
    fn the_calendar_follows_the_schedule() {
        let schedule =
            WorkingSchedule::try_new(8 * 60, 16 * 60, Workdays::from_mask(0b0000_0101).unwrap())
                .unwrap();
        let calendar = schedule.calendar();
        assert_eq!(
            calendar.workdays,
            [true, false, true, false, false, false, false]
        );
        assert_eq!(calendar.minutes_per_day, 480);
    }
}
