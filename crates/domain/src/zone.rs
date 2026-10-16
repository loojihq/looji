//! Time zones: how a wall-clock moment in a named zone becomes an instant, and back.
//!
//! The working-time calculation works in wall-clock minutes and never needs a zone. A zone only matters where
//! a wall-clock moment has to become an instant, or an instant has to be read as a wall-clock moment. Both
//! go through this module.
//!
//! Decisions this module enforces:
//! - The zone database is compiled into the app, and the host's zone files are never read. The same zone
//!   name gives the same rules on every machine, whatever machine zone it has.
//! - A wall-clock moment that happens twice, when the clocks go back, takes its first occurrence.
//! - A wall-clock moment that never happens, when the clocks go forward, moves to the first valid time after
//!   the gap: the instant the clocks jumped. A working day that starts at 02:30 on the spring change starts at
//!   03:00 local.
//! - Minute 1440 is the next day's midnight, as in `LocalMoment`.

use std::fmt;

use jiff::{civil, tz::AmbiguousOffset, tz::TimeZone};
use thiserror::Error;

use crate::{
    due_calculation::LocalMoment, task_fields::DueDate, time::Timestamp,
    working_schedule::WorkingSchedule,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ZoneError {
    #[error("\"{0}\" is not a time zone this app knows")]
    Unknown(String),
    #[error("that date and time can't be represented")]
    OutOfRange,
}

/// A named IANA zone, such as `Europe/London`. Its rules come from the database compiled into the app.
#[derive(Clone)]
pub struct Zone {
    name: String,
    tz: TimeZone,
}

impl Zone {
    /// Looks a zone up by its IANA name. An unknown name is refused rather than read as UTC.
    pub fn named(name: &str) -> Result<Self, ZoneError> {
        let tz = TimeZone::get(name).map_err(|_| ZoneError::Unknown(name.to_string()))?;
        Ok(Self {
            name: name.to_string(),
            tz,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Zone({})", self.name)
    }
}

/// The civil date and time for a moment. Minute 1440 is the next day's midnight.
fn wall_clock(at: LocalMoment) -> Result<civil::DateTime, ZoneError> {
    let (day, minute) = if at.minute == 1440 {
        (at.day.plus_days(1), 0)
    } else {
        (at.day, at.minute)
    };
    civil::DateTime::new(
        i16::try_from(day.year()).map_err(|_| ZoneError::OutOfRange)?,
        i8::try_from(day.month()).map_err(|_| ZoneError::OutOfRange)?,
        i8::try_from(day.day()).map_err(|_| ZoneError::OutOfRange)?,
        i8::try_from(minute / 60).map_err(|_| ZoneError::OutOfRange)?,
        i8::try_from(minute % 60).map_err(|_| ZoneError::OutOfRange)?,
        0,
        0,
    )
    .map_err(|_| ZoneError::OutOfRange)
}

/// The instant at which a wall-clock moment happens in `zone`.
///
/// A moment that happens twice takes its earlier instant. A moment that never happens takes the instant the
/// clocks jumped over it, which is the first valid time after the gap.
pub fn local_to_timestamp(zone: &Zone, at: LocalMoment) -> Result<Timestamp, ZoneError> {
    let wall = wall_clock(at)?;
    let ambiguous = zone.tz.to_ambiguous_timestamp(wall);
    let instant = match ambiguous.offset() {
        AmbiguousOffset::Gap { .. } => {
            // `later` reads the moment on the offset from before the jump. That instant is at or after the jump,
            // so the jump is the latest transition at or before it. `preceding` is strict, so one millisecond is
            // added to include a jump that lands exactly on that instant.
            let read_before_jump = ambiguous.later().map_err(|_| ZoneError::OutOfRange)?;
            let just_after =
                jiff::Timestamp::from_millisecond(read_before_jump.as_millisecond() + 1)
                    .map_err(|_| ZoneError::OutOfRange)?;
            zone.tz
                .preceding(just_after)
                .next()
                .map(|transition| transition.timestamp())
                .ok_or(ZoneError::OutOfRange)?
        }
        _ => ambiguous.earlier().map_err(|_| ZoneError::OutOfRange)?,
    };
    Ok(Timestamp::from_unix_millis(instant.as_millisecond()))
}

/// The wall-clock moment an instant reads as in `zone`, to the minute.
pub fn timestamp_to_local(zone: &Zone, at: Timestamp) -> Result<LocalMoment, ZoneError> {
    let instant =
        jiff::Timestamp::from_millisecond(at.unix_millis()).map_err(|_| ZoneError::OutOfRange)?;
    let wall = instant.to_zoned(zone.tz.clone()).datetime();
    let day = DueDate::new(
        i32::from(wall.year()),
        u32::try_from(wall.month()).map_err(|_| ZoneError::OutOfRange)?,
        u32::try_from(wall.day()).map_err(|_| ZoneError::OutOfRange)?,
    )
    .ok_or(ZoneError::OutOfRange)?;
    let minute = u16::try_from(wall.hour()).map_err(|_| ZoneError::OutOfRange)? * 60
        + u16::try_from(wall.minute()).map_err(|_| ZoneError::OutOfRange)?;
    Ok(LocalMoment::new(day, minute))
}

/// The instant a day's work begins under `schedule` in `zone`. If the schedule's start falls in a skipped
/// hour that day, the work begins at the first valid time after the gap.
pub fn work_start(
    schedule: &WorkingSchedule,
    day: DueDate,
    zone: &Zone,
) -> Result<Timestamp, ZoneError> {
    local_to_timestamp(zone, LocalMoment::new(day, schedule.start()))
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::{Zone, ZoneError, local_to_timestamp, timestamp_to_local, work_start};
    use crate::{
        due_calculation::{LocalMoment, add_working_minutes},
        task_fields::DueDate,
        time::Timestamp,
        working_schedule::{Workdays, WorkingSchedule},
    };

    /// New York springs forward on 8 March 2026 (02:00 to 03:00) and falls back on 1 November 2026 (02:00 to 01:00).
    const NEW_YORK: &str = "America/New_York";

    fn zone(name: &str) -> Zone {
        Zone::named(name).unwrap()
    }

    fn day(year: i32, month: u32, day: u32) -> DueDate {
        DueDate::new(year, month, day).unwrap()
    }

    fn local(year: i32, month: u32, date: u32, hour: u16, minute: u16) -> LocalMoment {
        LocalMoment::new(day(year, month, date), hour * 60 + minute)
    }

    /// An instant written as RFC 3339, so the expected values read the way a calendar would.
    fn instant(text: &str) -> Timestamp {
        let parsed: jiff::Timestamp = text.parse().unwrap();
        Timestamp::from_unix_millis(parsed.as_millisecond())
    }

    fn at(zone: &Zone, moment: LocalMoment) -> Timestamp {
        local_to_timestamp(zone, moment).unwrap()
    }

    // Daylight saving

    #[test]
    fn a_time_inside_the_skipped_hour_moves_to_the_first_valid_time_after_it() {
        // 02:30 never happens on 8 March 2026 in New York. The clocks jump from 02:00 EST to 03:00 EDT, which
        // is 07:00 UTC, so the work starts at 03:00 local.
        let ny = zone(NEW_YORK);
        assert_eq!(
            at(&ny, local(2026, 3, 8, 2, 30)),
            instant("2026-03-08T07:00:00Z")
        );
        assert_eq!(
            at(&ny, local(2026, 3, 8, 2, 59)),
            instant("2026-03-08T07:00:00Z")
        );
    }

    #[test]
    fn the_start_of_the_skipped_hour_also_moves_to_the_jump() {
        let ny = zone(NEW_YORK);
        assert_eq!(
            at(&ny, local(2026, 3, 8, 2, 0)),
            instant("2026-03-08T07:00:00Z")
        );
    }

    #[test]
    fn the_times_either_side_of_the_gap_are_not_moved() {
        let ny = zone(NEW_YORK);
        assert_eq!(
            at(&ny, local(2026, 3, 8, 1, 59)),
            instant("2026-03-08T06:59:00Z")
        );
        assert_eq!(
            at(&ny, local(2026, 3, 8, 3, 0)),
            instant("2026-03-08T07:00:00Z")
        );
        assert_eq!(
            at(&ny, local(2026, 3, 8, 3, 30)),
            instant("2026-03-08T07:30:00Z")
        );
    }

    #[test]
    fn a_repeated_hour_takes_its_first_occurrence() {
        // 01:30 happens twice on 1 November 2026. The first is on daylight time, which is 05:30 UTC.
        let ny = zone(NEW_YORK);
        assert_eq!(
            at(&ny, local(2026, 11, 1, 1, 30)),
            instant("2026-11-01T05:30:00Z")
        );
    }

    #[test]
    fn after_the_repeated_hour_the_clock_reads_standard_time() {
        let ny = zone(NEW_YORK);
        assert_eq!(
            at(&ny, local(2026, 11, 1, 2, 0)),
            instant("2026-11-01T07:00:00Z")
        );
    }

    #[test]
    fn a_spring_day_is_23_hours_long_and_an_autumn_day_is_25() {
        let ny = zone(NEW_YORK);
        let spring_start = at(&ny, local(2026, 3, 8, 0, 0));
        let spring_end = at(&ny, local(2026, 3, 9, 0, 0));
        let autumn_start = at(&ny, local(2026, 11, 1, 0, 0));
        let autumn_end = at(&ny, local(2026, 11, 2, 0, 0));
        assert_eq!(
            spring_end.unix_millis() - spring_start.unix_millis(),
            23 * 3_600_000
        );
        assert_eq!(
            autumn_end.unix_millis() - autumn_start.unix_millis(),
            25 * 3_600_000
        );
    }

    #[test]
    fn a_day_whose_work_starts_in_the_gap_begins_at_the_jump() {
        let ny = zone(NEW_YORK);
        let schedule =
            WorkingSchedule::try_new(2 * 60 + 30, 10 * 60, Workdays::MON_TO_FRI).unwrap();
        let on_the_change = work_start(&schedule, day(2026, 3, 8), &ny).unwrap();
        assert_eq!(on_the_change, instant("2026-03-08T07:00:00Z"));
        assert_eq!(
            timestamp_to_local(&ny, on_the_change).unwrap(),
            local(2026, 3, 8, 3, 0)
        );
        // The next day has no gap, so the same schedule starts at 02:30 on the wall clock.
        let after = work_start(&schedule, day(2026, 3, 9), &ny).unwrap();
        assert_eq!(after, at(&ny, local(2026, 3, 9, 2, 30)));
    }

    #[test]
    fn reading_a_time_and_writing_it_back_gives_the_same_time_outside_the_gap() {
        let ny = zone(NEW_YORK);
        for (year, month, date) in [(2026, 3, 7), (2026, 3, 9), (2026, 10, 31), (2026, 11, 2)] {
            for minute in (0..1440).step_by(15) {
                let moment = LocalMoment::new(day(year, month, date), minute);
                let back = timestamp_to_local(&ny, at(&ny, moment)).unwrap();
                assert_eq!(back, moment, "{moment:?} must survive a round trip");
            }
        }
    }

    #[test]
    fn a_due_date_is_the_same_on_a_clock_change_week_as_on_any_other_week() {
        // Due dates are days and working time is wall-clock time, so the clocks changing doesn't move a due
        // date. Friday 16:00 with five hours left: one hour on Friday, four on Monday.
        let schedule = WorkingSchedule::default();
        let spring = add_working_minutes(&schedule, local(2026, 3, 6, 16, 0), 300).unwrap();
        let autumn = add_working_minutes(&schedule, local(2026, 10, 16, 16, 0), 300).unwrap();
        assert_eq!(spring.day, day(2026, 3, 9));
        assert_eq!(autumn.day, day(2026, 10, 19));
        assert_eq!(spring.minute, 13 * 60);
    }

    // Midnight

    #[test]
    fn midnight_in_a_zone_with_a_half_hour_offset_is_a_different_instant_from_utc_midnight() {
        // Kolkata is 5 hours 30 minutes ahead of UTC, so its midnight is 18:30 UTC the day before.
        let kolkata = zone("Asia/Kolkata");
        assert_eq!(
            at(&kolkata, local(2026, 10, 16, 0, 0)),
            instant("2026-10-15T18:30:00Z")
        );
    }

    #[test]
    fn the_last_minute_before_midnight_and_midnight_itself_fall_on_different_days() {
        let kolkata = zone("Asia/Kolkata");
        let before = timestamp_to_local(&kolkata, instant("2026-10-15T18:29:59.999Z")).unwrap();
        let after = timestamp_to_local(&kolkata, instant("2026-10-15T18:30:00Z")).unwrap();
        assert_eq!(before, local(2026, 10, 15, 23, 59));
        assert_eq!(after, local(2026, 10, 16, 0, 0));
    }

    #[test]
    fn minute_1440_is_the_next_days_midnight_in_every_zone() {
        let kolkata = zone("Asia/Kolkata");
        let end_of_day = LocalMoment::new(day(2026, 10, 16), 1440);
        assert_eq!(
            at(&kolkata, end_of_day),
            at(&kolkata, local(2026, 10, 17, 0, 0))
        );
    }

    #[test]
    fn a_midnight_that_never_happens_moves_to_the_first_valid_time_after_it() {
        // Santiago's clocks jump from 00:00 to 01:00 on 6 September 2026, so midnight doesn't happen that day.
        // The first valid time is 01:00 at -03, which is 04:00 UTC.
        let santiago = zone("America/Santiago");
        let result = at(&santiago, local(2026, 9, 6, 0, 0));
        assert_eq!(result, instant("2026-09-06T04:00:00Z"));
        assert_eq!(
            timestamp_to_local(&santiago, result).unwrap(),
            local(2026, 9, 6, 1, 0)
        );
    }

    // Zones that differ from this machine's

    #[test]
    fn a_half_hour_zone_gives_its_own_instant() {
        // India is UTC+05:30 all year, so 09:00 there is 03:30 UTC.
        let kolkata = zone("Asia/Kolkata");
        assert_eq!(
            at(&kolkata, local(2026, 10, 16, 9, 0)),
            instant("2026-10-16T03:30:00Z")
        );
    }

    #[test]
    fn a_zone_ahead_of_utc_by_a_quarter_hour_gives_its_own_instant() {
        // The Chatham Islands are UTC+13:45 in January, so 09:00 on 15 January is 19:15 UTC on the 14th.
        let chatham = zone("Pacific/Chatham");
        assert_eq!(
            at(&chatham, local(2026, 1, 15, 9, 0)),
            instant("2026-01-14T19:15:00Z")
        );
    }

    #[test]
    fn a_zone_behind_utc_by_a_half_hour_gives_its_own_instant() {
        // Newfoundland is UTC-02:30 in daylight time, so 09:00 on 16 October is 11:30 UTC.
        let st_johns = zone("America/St_Johns");
        assert_eq!(
            at(&st_johns, local(2026, 10, 16, 9, 0)),
            instant("2026-10-16T11:30:00Z")
        );
    }

    #[test]
    fn the_same_wall_clock_time_is_a_different_instant_in_each_zone() {
        let moment = local(2026, 10, 16, 9, 0);
        let utc = at(&zone("UTC"), moment);
        assert_ne!(at(&zone("Asia/Kolkata"), moment), utc);
        assert_ne!(at(&zone(NEW_YORK), moment), utc);
    }

    #[test]
    fn an_unknown_zone_is_refused_rather_than_read_as_utc() {
        assert!(matches!(
            Zone::named("Mars/Olympus_Mons"),
            Err(ZoneError::Unknown(name)) if name == "Mars/Olympus_Mons"
        ));
    }

    #[test]
    fn every_zone_gives_a_valid_instant_for_every_hour_of_2026() {
        // A sweep over the year. A moment that happens gives back the same moment. A moment in a gap gives back
        // the first valid time after it, on the same day and within the length of the jump.
        for name in [
            NEW_YORK,
            "Europe/London",
            "Asia/Kolkata",
            "America/Santiago",
            "Pacific/Chatham",
            "America/St_Johns",
            "Australia/Lord_Howe",
        ] {
            let zone = zone(name);
            for ordinal in 0..365 {
                let date = DueDate::from_day_number(day(2026, 1, 1).day_number() + ordinal);
                for hour in 0..24 {
                    let moment = LocalMoment::new(date, hour * 60);
                    let back = timestamp_to_local(&zone, at(&zone, moment)).unwrap();
                    assert_eq!(back.day, moment.day, "{name} {moment:?}");
                    assert!(
                        back.minute >= moment.minute && back.minute - moment.minute <= 60,
                        "{name} {moment:?} read back as {back:?}"
                    );
                }
            }
        }
    }
}
