//! The date picker: a small trigger that shows a date, and a month calendar that opens under it. It serves the task
//! form's start and due dates and the planning window's target date. Picking a day sets the date, and Clear empties it.

use gpui::{prelude::*, *};

use super::WorkspaceView;
use super::card::civil_from_days;
use super::task_form::Field;
use crate::{components, theme::Theme};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// Which date a calendar sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DateTarget {
    TaskStart,
    TaskDue,
    PlanTarget,
}

/// A calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// The calendar while it's open: which date it sets, and the month it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Calendar {
    pub target: DateTarget,
    pub year: i32,
    pub month: u32,
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's days_from_civil).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let (year, month, day) = (i64::from(year), i64::from(month), i64::from(day));
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Monday is 0, as the week's columns are.
fn weekday(days: i64) -> usize {
    (days + 3).rem_euclid(7) as usize
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)) as u32
}

/// The date a `YYYY-MM-DD` value holds, if it holds one.
pub(super) fn parse(value: &str) -> Option<Day> {
    let mut parts = value.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    (day <= days_in_month(year, month)).then_some(Day { year, month, day })
}

fn format(day: Day) -> String {
    format!("{:04}-{:02}-{:02}", day.year, day.month, day.day)
}

/// Today's date. It's taken in UTC, so it can be a day off near midnight.
pub(super) fn today() -> Day {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    Day { year, month, day }
}

/// The date a month after today, as a `YYYY-MM-DD` value. A day the later month doesn't have is clamped to its last
/// day, so the 31st becomes the 30th in a thirty-day month.
pub(super) fn one_month_ahead() -> String {
    let now = today();
    let (year, month) = if now.month == 12 {
        (now.year + 1, 1)
    } else {
        (now.year, now.month + 1)
    };
    let day = now.day.min(days_in_month(year, month));
    format(Day { year, month, day })
}

/// The date as the trigger shows it, such as "18 Oct 2026".
fn label(day: Day) -> String {
    format!(
        "{} {} {}",
        day.day,
        SHORT_MONTHS[(day.month - 1) as usize],
        day.year
    )
}

/// The date field's control: its date, or a prompt while it's empty, and the calendar under it when it's the open one.
pub(super) fn date_field(
    target: DateTarget,
    value: &str,
    calendar: Option<Calendar>,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let open = calendar.is_some_and(|open| open.target == target);
    let chosen = parse(value);
    let shown = match chosen {
        Some(day) => div().child(label(day)),
        None => div()
            .text_color(theme.palette.text_secondary)
            .child("Pick a date"),
    };
    let id = match target {
        DateTarget::TaskStart => "task-start",
        DateTarget::TaskDue => "task-due",
        DateTarget::PlanTarget => "plan-date",
    };
    let trigger = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .h(px(32.))
        .px(theme.spacing.md)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(if open {
            theme.palette.accent
        } else {
            theme.palette.border_control
        })
        .bg(theme.palette.surface_raised)
        .text_size(theme.typography.body)
        .cursor_pointer()
        .child(shown)
        .child(components::icon(
            "calendar",
            15.,
            theme.palette.text_secondary,
        ))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_calendar(target, cx)));
    let mut item = div()
        .relative()
        // Presses on the control and its calendar stay with them, so the form doesn't close its menus.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);
    if let (true, Some(calendar)) = (open, calendar) {
        item = item.child(deferred(month_grid(calendar, chosen, theme, cx)).with_priority(1));
    }
    item.into_any_element()
}

/// The month calendar: the month's name with arrows to move between months, the weekdays, the days, and Clear and
/// Today.
fn month_grid(
    calendar: Calendar,
    chosen: Option<Day>,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let now = today();
    let first = days_from_civil(calendar.year, calendar.month, 1);
    let lead = weekday(first);
    let count = days_in_month(calendar.year, calendar.month) as usize;
    // Blank cells before the first day, and after the last, so the weeks are full rows.
    let mut cells: Vec<Option<u32>> = vec![None; lead];
    cells.extend((1..=count as u32).map(Some));
    while !cells.len().is_multiple_of(7) {
        cells.push(None);
    }

    let mut weeks = div().flex().flex_col().gap(px(2.));
    for (row_index, row) in cells.chunks(7).enumerate() {
        let mut line = div().flex().flex_row().gap(px(2.));
        for (column, cell) in row.iter().enumerate() {
            let index = row_index * 7 + column;
            line = line.child(day_cell(index, *cell, calendar, chosen, now, theme, cx));
        }
        weeks = weeks.child(line);
    }

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(nav_button("calendar-previous", "‹", -1, theme, cx))
        .child(div().font_weight(FontWeight::MEDIUM).child(format!(
            "{} {}",
            MONTHS[(calendar.month - 1) as usize],
            calendar.year
        )))
        .child(nav_button("calendar-next", "›", 1, theme, cx));

    let mut names = div().flex().flex_row().gap(px(2.));
    for name in WEEKDAYS {
        names = names.child(
            div()
                .w(px(30.))
                .flex()
                .justify_center()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(name),
        );
    }

    let footer =
        div()
            .flex()
            .flex_row()
            .justify_between()
            .pt(theme.spacing.xs)
            .border_t_1()
            .border_color(theme.palette.border)
            .child(
                components::button("calendar-clear", "Clear", components::Variant::Ghost, theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.pick_date(None, cx))),
            )
            .child(
                components::button(
                    "calendar-today",
                    "Today",
                    components::Variant::Secondary,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.pick_date(Some(format(now)), cx)
                })),
            );

    div()
        .absolute()
        .top(px(36.))
        .left_0()
        .w(px(252.))
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .p(theme.spacing.sm)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .shadow(theme.elevation.overlay.clone())
        .occlude()
        .child(header)
        .child(names)
        .child(weeks)
        .child(footer)
        .into_any_element()
}

fn nav_button(
    id: &'static str,
    glyph: &'static str,
    step: i32,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    div()
        .id(id)
        .w(px(28.))
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme.radius.md)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(glyph)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.shift_calendar(step, cx)))
        .into_any_element()
}

fn day_cell(
    index: usize,
    day: Option<u32>,
    calendar: Calendar,
    chosen: Option<Day>,
    now: Day,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let Some(number) = day else {
        return div().w(px(30.)).h(px(28.)).into_any_element();
    };
    let this_day = Day {
        year: calendar.year,
        month: calendar.month,
        day: number,
    };
    let selected = chosen == Some(this_day);
    let is_today = now == this_day;
    div()
        .id(("calendar-day", index as u64))
        .w(px(30.))
        .h(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme.radius.md)
        .cursor_pointer()
        .text_size(theme.typography.body)
        .when(selected, |cell| {
            cell.bg(theme.palette.accent)
                .text_color(theme.palette.on_accent)
        })
        .when(!selected, |cell| {
            cell.hover(|style| style.bg(theme.palette.surface_hover))
        })
        // Today is outlined, so it's findable when another day is chosen.
        .when(is_today && !selected, |cell| {
            cell.border_1().border_color(theme.palette.accent)
        })
        .child(number.to_string())
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.pick_date(Some(format(this_day)), cx)
        }))
        .into_any_element()
}

impl WorkspaceView {
    /// The text a target holds right now.
    fn date_text(&self, target: DateTarget) -> String {
        match target {
            DateTarget::TaskStart => self
                .task_dialog
                .as_ref()
                .map(|dialog| dialog.form.values.start.as_str().to_string())
                .unwrap_or_default(),
            DateTarget::TaskDue => self
                .task_dialog
                .as_ref()
                .map(|dialog| dialog.form.values.due.as_str().to_string())
                .unwrap_or_default(),
            DateTarget::PlanTarget => self
                .plan_wizard
                .as_ref()
                .map(|dialog| dialog.wizard.target_text().to_string())
                .unwrap_or_default(),
        }
    }

    /// Opens the calendar for a date, or closes it when it's the open one. The task form's other menus close, and
    /// its field takes the focus.
    pub(super) fn toggle_calendar(&mut self, target: DateTarget, cx: &mut Context<Self>) {
        if self.open_calendar.is_some_and(|open| open.target == target) {
            self.open_calendar = None;
            return cx.notify();
        }
        let shown = parse(&self.date_text(target)).unwrap_or_else(today);
        self.open_calendar = Some(Calendar {
            target,
            year: shown.year,
            month: shown.month,
        });
        if let Some(dialog) = self.task_dialog.as_mut() {
            dialog.priority_open = false;
            dialog.assignee_open = false;
            match target {
                DateTarget::TaskStart => dialog.form.focus(Field::Start),
                DateTarget::TaskDue => dialog.form.focus(Field::Due),
                DateTarget::PlanTarget => {}
            }
        }
        cx.notify();
    }

    /// Moves the open calendar a month forward or back.
    pub(super) fn shift_calendar(&mut self, step: i32, cx: &mut Context<Self>) {
        let Some(calendar) = self.open_calendar.as_mut() else {
            return;
        };
        let index = calendar.year * 12 + calendar.month as i32 - 1 + step;
        calendar.year = index.div_euclid(12);
        calendar.month = index.rem_euclid(12) as u32 + 1;
        cx.notify();
    }

    /// Sets the open calendar's date, or clears it, and closes the calendar.
    pub(super) fn pick_date(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        let Some(calendar) = self.open_calendar.take() else {
            return;
        };
        let value = value.unwrap_or_default();
        match calendar.target {
            DateTarget::TaskStart => {
                if let Some(dialog) = self.task_dialog.as_mut() {
                    dialog.form.values.start = value.into();
                    dialog.form.focus(Field::Start);
                }
            }
            DateTarget::TaskDue => {
                if let Some(dialog) = self.task_dialog.as_mut() {
                    dialog.form.values.due = value.into();
                    dialog.form.focus(Field::Due);
                }
            }
            DateTarget::PlanTarget => {
                if let Some(dialog) = self.plan_wizard.as_mut() {
                    dialog.wizard.set_target_text(&value);
                }
            }
        }
        cx.notify();
    }
}
