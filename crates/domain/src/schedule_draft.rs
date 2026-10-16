//! Working hours as the user edits them: a draft that's checked before it's saved, and the preview of
//! when work finishes under it.
//!
//! The draft holds times as minutes since midnight and days as the weekday mask. It keeps them as `i64`,
//! so the validator can report a value outside the day rather than having it cut to fit. The validator
//! is the one list of rules. The save path runs it too, so a draft that skips the screen is still refused.
//!
//! The preview uses the same calculator as a real due date. It starts from an anchor moment and chains
//! sample tasks: each one starts when the one before it ends. The rule, the calendar, and the working day
//! are all the calculator's, so the preview can't disagree with a stored date for the same inputs.

use crate::{
    DomainError,
    due_calculation::{LocalMoment, add_working_minutes},
    task::Estimate,
    task_fields::DueDate,
    working_schedule::{MINUTES_PER_DAY, ScheduleError, WorkingSchedule},
};

/// The estimate the preview uses: a task of five hours.
pub const SAMPLE_ESTIMATE_HOURS: f64 = 5.0;
/// How many due dates the preview shows.
pub const PREVIEW_COUNT: usize = 5;
/// What a time input says when it can't read what was typed. The draft keeps its last valid time.
pub const TIME_FORMAT_MESSAGE: &str = "Enter a time as HH:MM, such as 09:00.";

/// Which time a rule is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeField {
    Start,
    End,
}

/// One rule the draft breaks. Each has a message the user can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftIssue {
    /// No weekday is a working day.
    NoWorkdays,
    /// The day set has a bit that isn't a weekday.
    UnknownWorkdays,
    /// A time is before midnight or after 24:00.
    TimeOutOfRange(TimeField),
    /// The start is the same as the end or after it.
    StartNotBeforeEnd,
}

impl DraftIssue {
    pub fn message(self) -> &'static str {
        match self {
            DraftIssue::NoWorkdays => "Select at least one workday to save.",
            DraftIssue::UnknownWorkdays => "Only the seven days of the week can be workdays.",
            DraftIssue::TimeOutOfRange(TimeField::Start) => {
                "The start time must be between 00:00 and 24:00."
            }
            DraftIssue::TimeOutOfRange(TimeField::End) => {
                "The end time must be between 00:00 and 24:00."
            }
            DraftIssue::StartNotBeforeEnd => "The start time must be before the end time.",
        }
    }
}

/// The working hours as entered. `start` and `end` are minutes since midnight, and `workdays` is the
/// weekday mask with Monday as bit 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleDraft {
    pub start: i64,
    pub end: i64,
    pub workdays: u8,
}

impl ScheduleDraft {
    pub fn from_schedule(schedule: &WorkingSchedule) -> Self {
        Self {
            start: i64::from(schedule.start()),
            end: i64::from(schedule.end()),
            workdays: schedule.workdays().mask(),
        }
    }

    /// Every rule the draft breaks, in the order the screen reads them. An empty list means it's valid.
    pub fn issues(&self) -> Vec<DraftIssue> {
        let mut issues = Vec::new();
        if self.workdays == 0 {
            issues.push(DraftIssue::NoWorkdays);
        } else if self.workdays & !0b0111_1111 != 0 {
            issues.push(DraftIssue::UnknownWorkdays);
        }
        let max = i64::from(MINUTES_PER_DAY);
        let start_ok = (0..=max).contains(&self.start);
        let end_ok = (0..=max).contains(&self.end);
        if !start_ok {
            issues.push(DraftIssue::TimeOutOfRange(TimeField::Start));
        }
        if !end_ok {
            issues.push(DraftIssue::TimeOutOfRange(TimeField::End));
        }
        if start_ok && end_ok && self.start >= self.end {
            issues.push(DraftIssue::StartNotBeforeEnd);
        }
        issues
    }

    /// The checked schedule, or the rules it breaks. Nothing is built from a draft that breaks a rule.
    pub fn to_schedule(&self) -> Result<WorkingSchedule, Vec<DraftIssue>> {
        let issues = self.issues();
        if !issues.is_empty() {
            return Err(issues);
        }
        let start = u16::try_from(self.start).expect("checked to be within the day");
        let end = u16::try_from(self.end).expect("checked to be within the day");
        let workdays = crate::working_schedule::Workdays::from_mask(self.workdays)
            .expect("checked to have a weekday and no other bits");
        WorkingSchedule::try_new(start, end, workdays).map_err(|err| {
            // The checks above cover every case `try_new` refuses, so this is only reached if they drift apart.
            vec![match err {
                ScheduleError::NoWorkdays => DraftIssue::NoWorkdays,
                ScheduleError::UnknownWorkdayBits { .. } => DraftIssue::UnknownWorkdays,
                ScheduleError::TimeOutOfRange { .. } => DraftIssue::TimeOutOfRange(TimeField::End),
                _ => DraftIssue::StartNotBeforeEnd,
            }]
        })
    }
}

/// Reads a time typed as `H:MM` or `HH:MM`, from 00:00 to 24:00. A partial entry such as `1` or `1:` is refused,
/// so it can't reach the draft.
pub fn parse_time(text: &str) -> Option<u16> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if hours.len() > 2 || minutes.len() != 2 || !digits(hours) || !digits(minutes) {
        return None;
    }
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    if minutes > 59 {
        return None;
    }
    let total = hours * 60 + minutes;
    (total <= MINUTES_PER_DAY).then_some(total)
}

/// The due dates of the next `count` sample tasks, each starting when the one before it ends. Each date is
/// the day the calculator puts the finish on. The schedule was checked when it was built, so the calculator
/// has working time to count in.
pub fn preview_due_dates(
    schedule: &WorkingSchedule,
    anchor: LocalMoment,
    count: usize,
) -> Result<Vec<DueDate>, DomainError> {
    let minutes = Estimate::from_hours(SAMPLE_ESTIMATE_HOURS)?.minutes();
    let mut dates = Vec::with_capacity(count);
    let mut start = anchor;
    for _ in 0..count {
        let finish = add_working_minutes(schedule, start, minutes)?;
        dates.push(finish.day);
        start = finish;
    }
    Ok(dates)
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::{
        due_calculation::{LocalMoment, add_working_minutes},
        schedule_draft::{
            DraftIssue, PREVIEW_COUNT, ScheduleDraft, TimeField, parse_time, preview_due_dates,
        },
        task_fields::DueDate,
        working_schedule::{Workdays, WorkingSchedule},
    };

    const MON_TO_FRI: u8 = 0b0001_1111;

    fn day(year: i32, month: u32, day: u32) -> DueDate {
        DueDate::new(year, month, day).unwrap()
    }

    fn draft(start: i64, end: i64, workdays: u8) -> ScheduleDraft {
        ScheduleDraft {
            start,
            end,
            workdays,
        }
    }

    fn schedule(start: u16, end: u16, workdays: u8) -> WorkingSchedule {
        WorkingSchedule::try_new(start, end, Workdays::from_mask(workdays).unwrap()).unwrap()
    }

    #[test]
    fn zero_workdays_fails_with_the_message_the_save_button_shows() {
        let issues = draft(540, 1020, 0).issues();
        assert_eq!(issues, vec![DraftIssue::NoWorkdays]);
        assert_eq!(issues[0].message(), "Select at least one workday to save.");
    }

    #[test]
    fn a_start_equal_to_the_end_fails() {
        assert_eq!(
            draft(540, 540, MON_TO_FRI).issues(),
            vec![DraftIssue::StartNotBeforeEnd]
        );
    }

    #[test]
    fn a_start_after_the_end_fails() {
        assert_eq!(
            draft(1020, 540, MON_TO_FRI).issues(),
            vec![DraftIssue::StartNotBeforeEnd]
        );
    }

    #[test]
    fn a_valid_draft_passes_and_builds_the_schedule() {
        let valid = draft(540, 1020, MON_TO_FRI);
        assert!(valid.issues().is_empty());
        assert_eq!(valid.to_schedule(), Ok(WorkingSchedule::default()));
    }

    #[test]
    fn times_outside_the_day_fail_each_on_its_own_field() {
        assert_eq!(
            draft(-1, 1020, MON_TO_FRI).issues(),
            vec![DraftIssue::TimeOutOfRange(TimeField::Start)]
        );
        assert_eq!(
            draft(540, 1441, MON_TO_FRI).issues(),
            vec![DraftIssue::TimeOutOfRange(TimeField::End)]
        );
    }

    #[test]
    fn a_draft_with_several_broken_rules_reports_all_of_them() {
        assert_eq!(
            draft(540, 540, 0).issues(),
            vec![DraftIssue::NoWorkdays, DraftIssue::StartNotBeforeEnd]
        );
    }

    #[test]
    fn a_draft_that_breaks_a_rule_never_becomes_a_schedule() {
        assert!(draft(540, 540, MON_TO_FRI).to_schedule().is_err());
        assert!(draft(540, 1020, 0).to_schedule().is_err());
    }

    #[test]
    fn times_are_read_as_hours_and_minutes_and_partial_entries_are_refused() {
        assert_eq!(parse_time("9:00"), Some(540));
        assert_eq!(parse_time("09:00"), Some(540));
        assert_eq!(parse_time("23:59"), Some(1439));
        assert_eq!(parse_time("24:00"), Some(1440));
        assert_eq!(parse_time(" 17:30 "), Some(1050));
        for partial in [
            "", "1", "1:", "9:5", "09:", ":30", "24:30", "12:60", "123:00", "ab:cd", "9.30",
        ] {
            assert_eq!(
                parse_time(partial),
                None,
                "{partial:?} must not be read as a time"
            );
        }
    }

    #[test]
    fn a_friday_late_start_skips_the_weekend_for_the_first_date() {
        // Friday 16:00: an hour is left that day, and the other four fall on Monday.
        let anchor = LocalMoment::new(day(2026, 10, 16), 16 * 60);
        let dates = preview_due_dates(&WorkingSchedule::default(), anchor, 1).unwrap();
        assert_eq!(dates, vec![day(2026, 10, 19)]);
    }

    #[test]
    fn the_preview_chains_five_sample_tasks_each_starting_when_the_last_ended() {
        // Each five-hour task starts when the one before it ends. The first finishes at 14:00 on Friday,
        // so the second has three hours left that day and two on Monday.
        let anchor = LocalMoment::new(day(2026, 10, 16), 9 * 60);
        let dates = preview_due_dates(&WorkingSchedule::default(), anchor, PREVIEW_COUNT).unwrap();
        assert_eq!(
            dates,
            vec![
                day(2026, 10, 16),
                day(2026, 10, 19),
                day(2026, 10, 19),
                day(2026, 10, 20),
                day(2026, 10, 21),
            ]
        );
    }

    #[test]
    fn changing_a_workday_changes_the_preview() {
        let anchor = LocalMoment::new(day(2026, 10, 16), 9 * 60);
        let with_friday = preview_due_dates(&WorkingSchedule::default(), anchor, 1).unwrap();
        let without_friday =
            preview_due_dates(&schedule(540, 1020, 0b0000_1111), anchor, 1).unwrap();
        assert_eq!(with_friday, vec![day(2026, 10, 16)]);
        assert_eq!(without_friday, vec![day(2026, 10, 19)]);
        assert_ne!(with_friday, without_friday);
    }

    #[test]
    fn the_preview_is_the_calculator_output_for_the_same_inputs() {
        // The preview and a real move both call `add_working_minutes` with the same five hours.
        let anchor = LocalMoment::new(day(2026, 10, 14), 13 * 60);
        let schedule = WorkingSchedule::default();
        let real = add_working_minutes(&schedule, anchor, 300).unwrap();
        let preview = preview_due_dates(&schedule, anchor, 1).unwrap();
        assert_eq!(preview, vec![real.day]);
    }

    #[test]
    fn a_daylight_saving_day_is_an_ordinary_day_in_the_preview() {
        // Working times are wall-clock times with no zone, so the change of clocks doesn't alter the hours.
        // The day before and the day of the change give the same answer.
        let schedule = WorkingSchedule::default();
        // 29 March 2026 was the change of clocks in Europe. It was a Sunday, so the next working day is Monday 30.
        let friday = LocalMoment::new(day(2026, 3, 27), 9 * 60);
        let sunday_of_the_change = LocalMoment::new(day(2026, 3, 29), 9 * 60);
        assert_eq!(
            preview_due_dates(&schedule, friday, 1).unwrap(),
            vec![day(2026, 3, 27)]
        );
        assert_eq!(
            preview_due_dates(&schedule, sunday_of_the_change, 1).unwrap(),
            vec![day(2026, 3, 30)]
        );
    }
}
