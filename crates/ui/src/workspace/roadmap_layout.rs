//! The roadmap's arithmetic, with no window in it. Dates are whole calendar days, held as day numbers from
//! 1970-01-01. The layout works in whole days, and turns them into pixels only as the last step, so a bar
//! can't drift a day off the calendar through rounding.
//!
//! The calendar weeks start on Monday, everywhere. A task's span is inclusive: a task from the 16th to the
//! 18th covers three columns, and the 18th is a full column.

use looji_domain::{DueDate, Task, TeamMemberId};

/// The width of one day on the week scale, in pixels.
pub const WEEK_DAY_WIDTH: f32 = 40.;
/// The width of one day on the month scale, in pixels.
pub const MONTH_DAY_WIDTH: f32 = 12.;
/// How many days the week scale shows at once.
pub const WEEK_VIEW_DAYS: i64 = 56;
/// How many days the month scale shows at once.
pub const MONTH_VIEW_DAYS: i64 = 365;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scale {
    #[default]
    Week,
    Month,
}

impl Scale {
    pub fn day_width(self) -> f32 {
        match self {
            Scale::Week => WEEK_DAY_WIDTH,
            Scale::Month => MONTH_DAY_WIDTH,
        }
    }

    pub fn view_days(self) -> i64 {
        match self {
            Scale::Week => WEEK_VIEW_DAYS,
            Scale::Month => MONTH_VIEW_DAYS,
        }
    }
}

/// Monday is 0 and Sunday is 6. The week rule is the domain's, so the roadmap and the working-day count agree.
pub fn weekday(day: i64) -> i64 {
    looji_domain::weekday(DueDate::from_day_number(day)) as i64
}

pub fn is_weekend(day: i64) -> bool {
    weekday(day) >= 5
}

/// The days on screen: `count` days starting at day number `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub first: i64,
    pub count: i64,
}

impl Window {
    /// A window of the scale's length with `anchor` at its centre. Changing the scale keeps the anchor on
    /// screen, since both windows are centred on it.
    pub fn around(scale: Scale, anchor: i64) -> Self {
        let count = scale.view_days();
        Self {
            first: anchor - count / 2,
            count,
        }
    }

    pub fn last(self) -> i64 {
        self.first + self.count - 1
    }

    pub fn contains(self, day: i64) -> bool {
        (self.first..=self.last()).contains(&day)
    }

    pub fn width(self, scale: Scale) -> f32 {
        self.count as f32 * scale.day_width()
    }
}

/// How a task's dates place it on the roadmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// Both dates, with the due date on or after the start. The span runs from the start to the due date,
    /// inclusive.
    Planned { first: i64, last: i64 },
    /// Both dates, with the due date before the start. The span runs from the due date to the start, so
    /// the days between them are drawn, flagged. Neither date is moved.
    Inverted { first: i64, last: i64 },
    /// Not enough dates to draw a bar. The task goes in the tray.
    Unscheduled(TrayReason),
}

/// Why a task is in the tray instead of on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayReason {
    NoDates,
    OnlyStart,
    OnlyDue,
}

impl TrayReason {
    pub fn label(self) -> &'static str {
        match self {
            TrayReason::NoDates => "No dates",
            TrayReason::OnlyStart => "Needs a due date",
            TrayReason::OnlyDue => "Needs a start date",
        }
    }
}

/// Sorts a task's dates into the three cases. A task with only one of its dates is in the tray, since a
/// bar needs both ends.
pub fn schedule(start: Option<DueDate>, due: Option<DueDate>) -> Schedule {
    match (start, due) {
        (Some(start), Some(due)) => {
            let (start, due) = (start.day_number(), due.day_number());
            if due >= start {
                Schedule::Planned {
                    first: start,
                    last: due,
                }
            } else {
                Schedule::Inverted {
                    first: due,
                    last: start,
                }
            }
        }
        (Some(_), None) => Schedule::Unscheduled(TrayReason::OnlyStart),
        (None, Some(_)) => Schedule::Unscheduled(TrayReason::OnlyDue),
        (None, None) => Schedule::Unscheduled(TrayReason::NoDates),
    }
}

/// A bar on screen: where it starts and how far it runs, in columns and in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// The first column the bar covers, counted from the window's first day.
    pub first_column: i64,
    /// How many columns it covers. Always at least 1.
    pub columns: i64,
    pub left: f32,
    pub width: f32,
    pub inverted: bool,
}

/// The part of a span that falls in the window, as a bar. `None` when the span is entirely outside it.
/// A span that runs past an edge is cut at the edge, so the bar stays on screen.
pub fn bar_for(first: i64, last: i64, inverted: bool, window: Window, scale: Scale) -> Option<Bar> {
    let shown_first = first.max(window.first);
    let shown_last = last.min(window.last());
    if shown_first > shown_last {
        return None;
    }
    let first_column = shown_first - window.first;
    let columns = shown_last - shown_first + 1;
    let day_width = scale.day_width();
    Some(Bar {
        first_column,
        columns,
        left: first_column as f32 * day_width,
        width: columns as f32 * day_width,
        inverted,
    })
}

/// A run of days in one calendar month, as shown in the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthSpan {
    pub year: i32,
    pub month: u32,
    pub first_column: i64,
    pub columns: i64,
}

/// The months a window touches, split at each month's first day. The header draws one label per span.
pub fn month_spans(window: Window) -> Vec<MonthSpan> {
    let mut spans: Vec<MonthSpan> = Vec::new();
    for offset in 0..window.count {
        let date = DueDate::from_day_number(window.first + offset);
        match spans.last_mut() {
            Some(span) if span.year == date.year() && span.month == date.month() => {
                span.columns += 1;
            }
            _ => spans.push(MonthSpan {
                year: date.year(),
                month: date.month(),
                first_column: offset,
                columns: 1,
            }),
        }
    }
    spans
}

/// How the roadmap groups its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    #[default]
    Assignee,
    Tag,
}

/// The row a task sits under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    /// A team member, in the order they were added.
    Member(TeamMemberId),
    Unassigned,
    /// A tag. A task with several tags sits under the first one, alphabetically, so it's counted once.
    Tag(String),
    NoTag,
}

impl GroupKey {
    /// The rule for which row a task belongs to, by the grouping. Tags are sorted in a task, so the first
    /// tag is the alphabetically first one.
    pub fn of(task: &Task, grouping: Grouping) -> Self {
        match grouping {
            Grouping::Assignee => match task.assignee_id() {
                Some(id) => GroupKey::Member(id),
                None => GroupKey::Unassigned,
            },
            Grouping::Tag => match task.tags().first() {
                Some(tag) => GroupKey::Tag(tag.clone()),
                None => GroupKey::NoTag,
            },
        }
    }
}

/// The rows of a roadmap: each group, with how many of its tasks are on the timeline and how many are in
/// the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCount {
    pub key: GroupKey,
    pub scheduled: usize,
    pub unscheduled: usize,
}

/// Counts tasks per group. Groups with no tasks don't appear. The order is given by `member_order` for
/// assignees (team order), then unassigned last; for tags it's alphabetical, then no tag last.
pub fn group_counts(
    tasks: &[Task],
    grouping: Grouping,
    member_order: &[TeamMemberId],
) -> Vec<GroupCount> {
    let mut counts: Vec<GroupCount> = Vec::new();
    for task in tasks {
        let key = GroupKey::of(task, grouping);
        let scheduled = matches!(
            schedule(task.start_on(), task.due_on()),
            Schedule::Planned { .. } | Schedule::Inverted { .. }
        );
        let index = match counts.iter().position(|count| count.key == key) {
            Some(index) => index,
            None => {
                counts.push(GroupCount {
                    key,
                    scheduled: 0,
                    unscheduled: 0,
                });
                counts.len() - 1
            }
        };
        if scheduled {
            counts[index].scheduled += 1;
        } else {
            counts[index].unscheduled += 1;
        }
    }
    counts.sort_by_key(|count| group_rank(&count.key, member_order));
    counts
}

fn group_rank(key: &GroupKey, member_order: &[TeamMemberId]) -> (u8, usize, String) {
    match key {
        GroupKey::Member(id) => (
            0,
            member_order
                .iter()
                .position(|member| member == id)
                .unwrap_or(usize::MAX - 1),
            String::new(),
        ),
        GroupKey::Unassigned => (1, 0, String::new()),
        GroupKey::Tag(tag) => (0, 0, tag.clone()),
        GroupKey::NoTag => (1, 0, String::new()),
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{DueDate, Position, ProjectId, Status, Task, TeamMemberId};

    use super::{
        GroupKey, Grouping, MONTH_DAY_WIDTH, Scale, Schedule, TrayReason, Window, bar_for,
        group_counts, is_weekend, month_spans, schedule, weekday,
    };

    fn day(year: i32, month: u32, day: u32) -> i64 {
        DueDate::new(year, month, day).unwrap().day_number()
    }

    fn date(year: i32, month: u32, day: u32) -> Option<DueDate> {
        DueDate::new(year, month, day)
    }

    fn task(start: Option<DueDate>, due: Option<DueDate>, tags: &[&str]) -> Task {
        let mut task = Task::new(
            ProjectId::new(),
            "Task",
            Status::Todo,
            Position::new("a0").unwrap(),
        )
        .unwrap();
        task.set_start_on(start);
        task.set_due_on(due);
        for tag in tags {
            task.add_tag(tag).unwrap();
        }
        task
    }

    #[test]
    fn a_bar_from_the_16th_to_the_18th_is_three_columns_starting_on_the_16th() {
        // Window starts on 2026-10-14, a Wednesday.
        let window = Window {
            first: day(2026, 10, 14),
            count: 14,
        };
        let bar = bar_for(
            day(2026, 10, 16),
            day(2026, 10, 18),
            false,
            window,
            Scale::Week,
        )
        .expect("the bar is inside the window");

        assert_eq!(bar.first_column, 2, "two days after the window's first day");
        assert_eq!(bar.columns, 3, "the 16th, 17th, and 18th, inclusive");
        assert_eq!(bar.left, 2.0 * Scale::Week.day_width());
        assert_eq!(bar.width, 3.0 * Scale::Week.day_width());
    }

    #[test]
    fn a_one_day_task_is_one_column() {
        let window = Window {
            first: day(2026, 10, 1),
            count: 30,
        };
        let bar = bar_for(
            day(2026, 10, 10),
            day(2026, 10, 10),
            false,
            window,
            Scale::Month,
        )
        .unwrap();
        assert_eq!(bar.columns, 1);
        assert_eq!(bar.width, MONTH_DAY_WIDTH);
    }

    #[test]
    fn a_span_across_a_month_boundary_counts_every_day() {
        let window = Window {
            first: day(2026, 10, 25),
            count: 14,
        };
        let bar = bar_for(
            day(2026, 10, 30),
            day(2026, 11, 2),
            false,
            window,
            Scale::Week,
        )
        .unwrap();
        assert_eq!(bar.columns, 4, "30, 31, 1, and 2");
    }

    #[test]
    fn a_span_across_the_year_boundary_counts_every_day() {
        let window = Window {
            first: day(2026, 12, 25),
            count: 14,
        };
        let bar = bar_for(
            day(2026, 12, 31),
            day(2027, 1, 2),
            false,
            window,
            Scale::Week,
        )
        .unwrap();
        assert_eq!(bar.columns, 3, "31 December, then 1 and 2 January");
    }

    #[test]
    fn a_span_over_a_leap_day_has_the_extra_day() {
        let window = Window {
            first: day(2028, 2, 20),
            count: 20,
        };
        let bar = bar_for(
            day(2028, 2, 28),
            day(2028, 3, 1),
            false,
            window,
            Scale::Week,
        )
        .unwrap();
        assert_eq!(bar.columns, 3, "28 February, 29 February, and 1 March");
    }

    #[test]
    fn a_span_in_a_common_year_has_no_leap_day() {
        let window = Window {
            first: day(2026, 2, 20),
            count: 20,
        };
        let bar = bar_for(
            day(2026, 2, 28),
            day(2026, 3, 1),
            false,
            window,
            Scale::Week,
        )
        .unwrap();
        assert_eq!(bar.columns, 2, "28 February and 1 March");
    }

    #[test]
    fn a_span_wholly_outside_the_window_has_no_bar() {
        let window = Window {
            first: day(2026, 10, 1),
            count: 7,
        };
        assert!(
            bar_for(
                day(2026, 11, 1),
                day(2026, 11, 3),
                false,
                window,
                Scale::Week
            )
            .is_none()
        );
    }

    #[test]
    fn a_span_that_runs_past_an_edge_is_cut_at_the_edge() {
        let window = Window {
            first: day(2026, 10, 1),
            count: 7,
        };
        let bar = bar_for(
            day(2026, 9, 28),
            day(2026, 10, 3),
            false,
            window,
            Scale::Week,
        )
        .unwrap();
        assert_eq!(bar.first_column, 0);
        assert_eq!(bar.columns, 3, "only 1 to 3 October are on screen");
    }

    #[test]
    fn a_week_is_monday_to_sunday() {
        // 2000-01-01 was a Saturday, and 2026-10-12 is a Monday.
        assert_eq!(weekday(day(2000, 1, 1)), 5);
        assert_eq!(weekday(day(2026, 10, 12)), 0);
        assert!(is_weekend(day(2026, 10, 17)), "a Saturday");
        assert!(is_weekend(day(2026, 10, 18)), "a Sunday");
        assert!(!is_weekend(day(2026, 10, 16)), "a Friday");
    }

    #[test]
    fn month_spans_split_at_each_month_boundary() {
        let window = Window {
            first: day(2026, 10, 29),
            count: 6,
        };
        let spans = month_spans(window);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            (spans[0].month, spans[0].columns),
            (10, 3),
            "29, 30, and 31 October"
        );
        assert_eq!(
            (spans[1].month, spans[1].columns),
            (11, 3),
            "1 to 3 November"
        );
        assert_eq!(spans[1].first_column, 3);
    }

    #[test]
    fn a_leap_day_is_its_own_column_in_february() {
        let window = Window {
            first: day(2028, 2, 27),
            count: 4,
        };
        let spans = month_spans(window);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].columns, 3, "27, 28, and 29 February");
        assert_eq!(spans[1].month, 3);
    }

    #[test]
    fn a_due_date_before_the_start_is_inverted_and_not_swapped() {
        let schedule = schedule(date(2026, 10, 18), date(2026, 10, 16));
        assert_eq!(
            schedule,
            Schedule::Inverted {
                first: day(2026, 10, 16),
                last: day(2026, 10, 18),
            }
        );
    }

    #[test]
    fn a_due_date_equal_to_the_start_is_planned_for_one_day() {
        assert_eq!(
            schedule(date(2026, 10, 16), date(2026, 10, 16)),
            Schedule::Planned {
                first: day(2026, 10, 16),
                last: day(2026, 10, 16),
            }
        );
    }

    #[test]
    fn a_task_with_one_date_or_none_goes_in_the_tray_with_the_reason() {
        assert_eq!(
            schedule(date(2026, 10, 16), None),
            Schedule::Unscheduled(TrayReason::OnlyStart)
        );
        assert_eq!(
            schedule(None, date(2026, 10, 16)),
            Schedule::Unscheduled(TrayReason::OnlyDue)
        );
        assert_eq!(
            schedule(None, None),
            Schedule::Unscheduled(TrayReason::NoDates)
        );
    }

    #[test]
    fn the_tray_split_and_the_group_counts_agree() {
        let member = TeamMemberId::new();
        let mut planned = task(date(2026, 10, 1), date(2026, 10, 5), &["web"]);
        planned.assign(Some(member));
        let inverted = task(date(2026, 10, 9), date(2026, 10, 3), &["web"]);
        let loose = task(None, None, &[]);
        let half = task(date(2026, 10, 1), None, &["bug"]);
        let tasks = [planned, inverted, loose, half];

        let by_assignee = group_counts(&tasks, Grouping::Assignee, &[member]);
        assert_eq!(
            by_assignee,
            [
                super::GroupCount {
                    key: GroupKey::Member(member),
                    scheduled: 1,
                    unscheduled: 0,
                },
                super::GroupCount {
                    key: GroupKey::Unassigned,
                    scheduled: 1,
                    unscheduled: 2,
                },
            ],
            "an inverted range is still on the timeline, and a half-dated task is in the tray"
        );

        let by_tag = group_counts(&tasks, Grouping::Tag, &[]);
        let total: usize = by_tag
            .iter()
            .map(|count| count.scheduled + count.unscheduled)
            .sum();
        assert_eq!(total, tasks.len(), "every task is in exactly one group");
        assert_eq!(
            by_tag.last().unwrap().key,
            GroupKey::NoTag,
            "no tag comes last"
        );
    }

    #[test]
    fn a_task_with_several_tags_is_counted_under_its_first_tag_only() {
        let tasks = [task(date(2026, 10, 1), date(2026, 10, 2), &["web", "bug"])];
        let counts = group_counts(&tasks, Grouping::Tag, &[]);
        assert_eq!(counts.len(), 1);
        assert_eq!(
            counts[0].key,
            GroupKey::Tag("bug".to_string()),
            "alphabetically first"
        );
        assert_eq!(counts[0].scheduled, 1);
    }
}
