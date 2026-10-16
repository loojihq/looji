//! The workload view: for a period, how much estimated work each member carries against their working time.
//! The numbers come from the domain's `workload`, so this file only chooses the period and draws rows.
//!
//! The working calendar is the project's own working schedule, so the view is only as right as the schedule it
//! was given. The schedule is shown on screen.

use gpui::{prelude::*, *};
use looji_domain::{
    DueDate, OVERLOAD_PERCENT, Owner, Period, WorkingCalendar, WorkloadRow, format_hours, workload,
};

use super::WorkspaceView;
use super::members;
use crate::theme::Theme;

const BAR_WIDTH: f32 = 240.;
const LABEL_WIDTH: f32 = 200.;

/// The project's calendar, from its working schedule. A project with no selection gets the default schedule.
fn project_calendar(view: &WorkspaceView) -> WorkingCalendar {
    view.projects
        .active()
        .map(|project| project.working_schedule())
        .unwrap_or_default()
        .calendar()
}

/// The schedule in words, for the view: its weekdays and its hours.
fn describe_schedule(view: &WorkspaceView) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let schedule = view
        .projects
        .active()
        .map(|project| project.working_schedule())
        .unwrap_or_default();
    let days: Vec<&str> = (0..7)
        .filter(|day| schedule.workdays().contains(*day))
        .map(|day| NAMES[day])
        .collect();
    format!(
        "{}, {}–{}",
        days.join(" "),
        looji_domain::format_time(schedule.start()),
        looji_domain::format_time(schedule.end())
    )
}

/// The period the view shows, and whether done tasks count. `None` means this week, so the default
/// follows the calendar.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct WorkloadState {
    pub period: Option<Period>,
    pub include_done: bool,
}

impl WorkloadState {
    pub fn period(&self) -> Period {
        self.period
            .unwrap_or_else(|| Period::week_containing(DueDate::today_utc()))
    }
}

impl WorkspaceView {
    pub(super) fn set_workload_period(&mut self, period: Option<Period>, cx: &mut Context<Self>) {
        self.workload.period = period;
        cx.notify();
    }

    /// Moves the period by its own length, so "Later" shows the next period of the same size.
    pub(super) fn shift_workload_period(&mut self, direction: i64, cx: &mut Context<Self>) {
        let current = self.workload.period();
        self.workload.period = Some(current.shifted(direction * current.days()));
        cx.notify();
    }

    pub(super) fn toggle_workload_done(&mut self, cx: &mut Context<Self>) {
        self.workload.include_done = !self.workload.include_done;
        cx.notify();
    }
}

/// The period for "Next two weeks": the fortnight starting the Monday after this week.
fn next_two_weeks() -> Period {
    let this_week = Period::week_containing(DueDate::today_utc());
    Period {
        first: this_week.last.plus_days(1),
        last: this_week.last.plus_days(14),
    }
}

/// The period for "This month", from its first day to its last.
fn this_month() -> Period {
    let today = DueDate::today_utc();
    let first = DueDate::new(today.year(), today.month(), 1).expect("the first day exists");
    let (next_year, next_month) = if today.month() == 12 {
        (today.year() + 1, 1)
    } else {
        (today.year(), today.month() + 1)
    };
    let next_first = DueDate::new(next_year, next_month, 1).expect("the first day exists");
    Period {
        first,
        last: next_first.plus_days(-1),
    }
}

/// The workload view: the period controls, the calendar and scope note, one row per owner, and the rows.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let period = view.workload.period();
    let calendar = project_calendar(view);
    let members: Vec<_> = view.members.iter().map(|member| member.id()).collect();
    let report = workload(
        &view.board,
        &members,
        &calendar,
        period,
        view.workload.include_done,
    );
    let entries = members::entries(&view.members);
    let working_days = looji_domain::working_days(&calendar, period);
    let hours_per_day = format_hours(i64::from(calendar.minutes_per_day));
    let unestimated: u32 = report.rows.iter().map(|row| row.unestimated_tasks).sum();

    let mut rows = div().flex().flex_col().gap(theme.spacing.xs);
    for row in &report.rows {
        rows = rows.child(row_view(row, &entries, theme));
    }
    if report.rows.is_empty() {
        rows =
            rows.child(div().text_color(theme.palette.text_secondary).child(
                "No team members yet. Add people in the team dialog to see their workload.",
            ));
    }

    div()
        .id("workload-body")
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&view.scroll_workload)
        .gap(theme.spacing.md)
        .px(theme.spacing.lg)
        .py(theme.spacing.md)
        .child(controls(view, period, theme, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!(
                    "{working_days} working days · {hours_per_day} a day · working hours {}",
                    describe_schedule(view)
                ))
                .child(format!(
                    "Counts {} tasks due in this period. Not counted: {} due outside it, {} with no due date. {} counted tasks have no estimate and count as 0h.",
                    report.in_period,
                    report.outside_period,
                    report.undated,
                    unestimated,
                )),
        )
        .child(rows)
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!(
                    "Overloaded means more than {OVERLOAD_PERCENT}% of working time. Unassigned work has no one to measure against, so it shows hours only. Board filters and search don't change these numbers."
                )),
        )
        .into_any_element()
}

fn controls(
    view: &WorkspaceView,
    period: Period,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let preset =
        |label: &'static str, id: u64, target: Option<Period>| {
            div()
                .id(("workload-preset", id))
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .text_size(theme.typography.caption)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_workload_period(target, cx)
                }))
        };
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(theme.spacing.sm)
        .child(div().text_size(theme.typography.title).child(format!(
            "{} – {}",
            label(period.first),
            label(period.last)
        )))
        .child(preset("This week", 0, None))
        .child(preset("Next two weeks", 1, Some(next_two_weeks())))
        .child(preset("This month", 2, Some(this_month())))
        .child(
            div()
                .id("workload-earlier")
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .cursor_pointer()
                .child("Earlier")
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.shift_workload_period(-1, cx)),
                ),
        )
        .child(
            div()
                .id("workload-later")
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .cursor_pointer()
                .child("Later")
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.shift_workload_period(1, cx)),
                ),
        )
        .child(
            div()
                .id("workload-done")
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border_control)
                .text_size(theme.typography.caption)
                .cursor_pointer()
                .when(view.workload.include_done, |button| {
                    button.bg(theme.palette.surface_selected)
                })
                .child(if view.workload.include_done {
                    "Including done"
                } else {
                    "Done left out"
                })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_workload_done(cx))),
        )
        .into_any_element()
}

/// One row: the owner, a bar against their working time, and the numbers. Unassigned has no bar, since it
/// has no working time to be compared with.
fn row_view(row: &WorkloadRow, entries: &[members::Entry], theme: &Theme) -> impl IntoElement {
    let (name, mark) = match row.owner {
        Owner::Member(id) => entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| {
                (
                    entry.label.clone(),
                    Some((entry.initials.clone(), entry.colour)),
                )
            })
            .unwrap_or_else(|| ("Removed member".to_string(), None)),
        Owner::Removed(_) => ("Removed member".to_string(), None),
        Owner::Unassigned => ("Unassigned".to_string(), None),
    };
    let estimate = format_hours(row.estimated_minutes);
    let counts = format!(
        "{} task{}{}",
        row.tasks,
        if row.tasks == 1 { "" } else { "s" },
        match row.unestimated_tasks {
            0 => String::new(),
            count => format!(" · {count} without estimate"),
        }
    );

    let label = div()
        .w(px(LABEL_WIDTH))
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.xs)
        .child(match mark {
            Some((initials, colour)) => {
                super::fields::member_mark(&initials, colour, px(22.), theme).into_any_element()
            }
            None => div().into_any_element(),
        })
        .child(div().truncate().child(name));

    let body: AnyElement = match row.available_minutes {
        // Unassigned or removed work: hours only.
        None => div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(estimate))
            .child(div().text_color(theme.palette.text_secondary).child(counts))
            .into_any_element(),
        Some(0) => div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child("No working days in this period"),
            )
            .child(div().child(format!("{estimate} estimated")))
            .when(row.overloaded(), |line| {
                line.child(
                    div()
                        .text_color(theme.palette.danger)
                        .child("work with no working time"),
                )
            })
            .into_any_element(),
        Some(available) => {
            let percent = row.percent().unwrap_or(0);
            let filled = (percent.clamp(0, 100) as f32 / 100.) * BAR_WIDTH;
            let fill = if row.overloaded() {
                theme.palette.danger.opacity(0.7)
            } else {
                theme.palette.accent
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(
                    div()
                        .relative()
                        .w(px(BAR_WIDTH))
                        .h(px(12.))
                        .rounded(theme.radius.sm)
                        .bg(theme.palette.surface_selected)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(px(filled))
                                .rounded(theme.radius.sm)
                                .bg(fill),
                        ),
                )
                .child(div().child(format!("{} / {}", estimate, format_hours(available))))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if row.overloaded() {
                            theme.palette.danger
                        } else {
                            theme.palette.text_primary
                        })
                        .child(format!("{percent}%")),
                )
                .when(row.overloaded(), |line| {
                    line.child(
                        div()
                            .text_size(theme.typography.caption)
                            .text_color(theme.palette.danger)
                            .child("Overloaded"),
                    )
                })
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(counts),
                )
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .py(theme.spacing.xs)
        .border_b_1()
        .border_color(theme.palette.border)
        .child(label)
        .child(body)
}

fn label(day: DueDate) -> String {
    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {} {} {}",
        WEEKDAYS[looji_domain::weekday(day)],
        day.day(),
        MONTHS[(day.month() - 1) as usize],
        day.year()
    )
}
