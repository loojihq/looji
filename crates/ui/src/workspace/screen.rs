//! The workspace screens, Today and Week. They gather tasks from every project in the workspace, so
//! they answer what's due without opening each board. A task opens its project and its panel.

use gpui::{prelude::*, *};
use looji_domain::{DueDate, ProjectId, Task};

use super::card::civil_from_days;
use super::project_state::load_board;
use super::workload_view::project_calendar;
use super::{WorkspaceView, store};
use crate::{components, theme::Theme};

const FULL_DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const SHORT_DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Which workspace screen is showing, in place of a project's board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Today,
    Week,
    /// The active workspace's own screen: its name, icon, projects, and delete.
    Workspace,
}

/// A task with the project it belongs to, as the screens list it.
#[derive(Clone)]
pub struct Entry {
    pub project: ProjectId,
    pub project_name: String,
    pub task: Task,
}

/// Monday is 0. 1970-01-01 was a Thursday, which is index 3.
fn weekday(day_number: i64) -> usize {
    (day_number + 3).rem_euclid(7) as usize
}

/// The date as the screens show it, such as "Friday, Oct 17".
fn long_date(day_number: i64) -> String {
    let (_, month, day) = civil_from_days(day_number);
    format!(
        "{}, {} {}",
        FULL_DAYS[weekday(day_number)],
        MONTHS[(month - 1) as usize],
        day
    )
}

/// The short label for a week column, such as "Tue 20".
fn short_date(day_number: i64) -> String {
    let (_, _, day) = civil_from_days(day_number);
    format!("{} {}", SHORT_DAYS[weekday(day_number)], day)
}

/// The day a task is due, as a day number, when it has a due date.
fn due_day(entry: &Entry) -> Option<i64> {
    entry.task.due_on().map(|due| due.day_number())
}

impl WorkspaceView {
    /// Opens a workspace screen. Every project's tasks are gathered here, so the screen shows the
    /// whole workspace rather than one board.
    pub(super) fn open_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        // Preferences reads its settings directly, so it gathers no tasks.
        if matches!(screen, Screen::Workspace) {
            self.screen = Some(screen);
            return cx.notify();
        }
        self.screen = Some(screen);
        self.gather_screen_tasks(cx);
        cx.notify();
    }

    /// Reads every project's tasks into the Today and Week screens. Called on open and after each
    /// board reload, so a move or an edit shows up on those screens too.
    pub(super) fn gather_screen_tasks(&mut self, cx: &mut Context<Self>) {
        // Always gathered, not only while a screen is open, because the sidebar's Today count reads it.
        let mut entries = Vec::new();
        for project in self.projects.items() {
            match load_board(store(cx), project.id()) {
                Ok(tasks) => entries.extend(tasks.into_iter().map(|task| Entry {
                    project: project.id(),
                    project_name: project.name().to_string(),
                    task,
                })),
                Err(err) => {
                    self.status = Some(format!("Could not load {}: {err}", project.name()));
                }
            }
        }
        self.screen_entries = entries;
    }

    /// How many open tasks need attention: due today, and overdue, each counted only when its
    /// notification setting is on.
    pub(super) fn attention_count(&self, prefs: &looji_domain::Preferences) -> usize {
        let today = DueDate::today_utc().day_number();
        self.screen_entries
            .iter()
            .filter(|entry| entry.task.status() != looji_domain::Status::Done)
            .filter(|entry| match due_day(entry) {
                Some(day) if day == today => prefs.notify_due_today,
                Some(day) if day < today => prefs.notify_overdue,
                _ => false,
            })
            .count()
    }

    /// Opens a task from a screen: its project is selected, then its panel opens.
    fn open_screen_task(&mut self, entry: Entry, cx: &mut Context<Self>) {
        self.select_project(entry.project, cx);
        self.open_panel(entry.task.id(), cx);
    }
}

/// The title block that replaces the project header while a screen is open.
pub(super) fn header(view: &WorkspaceView, screen: Screen, theme: &Theme) -> AnyElement {
    let workspace_name = view.list.active().name().to_string();
    let (title, subtitle): (String, String) = match screen {
        Screen::Today => (
            "Today".to_string(),
            long_date(DueDate::today_utc().day_number()),
        ),
        Screen::Week => (
            "Plan the week".to_string(),
            "The next seven days".to_string(),
        ),
        Screen::Workspace => (
            workspace_name,
            "Set up the workspace, then add a project".to_string(),
        ),
    };
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .px(theme.spacing.lg)
        .pt(theme.spacing.md)
        .pb(theme.spacing.sm)
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(subtitle),
        )
        .child(
            div()
                .mt(theme.spacing.xs)
                .text_size(theme.typography.heading)
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .into_any_element()
}

/// The body of a screen, in place of a board.
pub(super) fn render(
    view: &WorkspaceView,
    screen: Screen,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    match screen {
        Screen::Today => today(view, theme, cx),
        Screen::Week => week(view, theme, cx),
        Screen::Workspace => super::workspace_setup::render(view, theme, cx),
    }
}

/// One task as a row on Today: its completion circle, title, project, and estimate.
fn task_row(
    index: usize,
    entry: &Entry,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let entry = entry.clone();
    let estimate = entry
        .task
        .estimate()
        .map(|estimate| format!("{} min", estimate.minutes()))
        .unwrap_or_default();
    div()
        .id(("screen-row", index as u64))
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .py(theme.spacing.sm)
        .border_b_1()
        .border_color(theme.palette.border)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(components::checkbox(
            entry.task.status() == looji_domain::Status::Done,
            theme,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(entry.task.title().to_string()),
        )
        .child(
            div()
                .flex_none()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(entry.project_name.clone()),
        )
        .child(
            div()
                .w(px(56.))
                .flex_none()
                .flex()
                .justify_end()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(estimate),
        )
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_screen_task(entry.clone(), cx)
            }),
        )
        .into_any_element()
}

fn today(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let today = DueDate::today_utc().day_number();
    let open: Vec<(usize, &Entry)> = view
        .screen_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.task.status() != looji_domain::Status::Done)
        .collect();
    let overdue: Vec<_> = open
        .iter()
        .copied()
        .filter(|(_, entry)| due_day(entry).is_some_and(|day| day < today))
        .collect();
    let due_today: Vec<_> = open
        .iter()
        .copied()
        .filter(|(_, entry)| due_day(entry) == Some(today))
        .collect();

    // Planned time is what's due today or overdue. Available time is one working day of the open
    // project's calendar.
    let planned: i64 = overdue
        .iter()
        .chain(due_today.iter())
        .map(|(_, entry)| {
            entry
                .task
                .estimate()
                .map_or(0, |estimate| i64::from(estimate.minutes()))
        })
        .sum();
    let available = i64::from(project_calendar(view).minutes_per_day);
    let fraction = if available > 0 {
        planned as f32 / available as f32
    } else {
        0.
    };
    let count = overdue.len() + due_today.len();

    let mut body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .id("screen-body")
        .overflow_y_scroll()
        .px(theme.spacing.lg)
        .pb(theme.spacing.lg)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.spacing.sm)
                .px(theme.spacing.md)
                .py(theme.spacing.sm)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .text_color(theme.palette.text_secondary)
                        .child(format!(
                            "{count} tasks · {} planned",
                            looji_domain::format_hours(planned)
                        ))
                        .child(format!(
                            "of {} available",
                            looji_domain::format_hours(available)
                        )),
                )
                .child(components::progress(fraction, theme)),
        );

    if !overdue.is_empty() {
        body = body.child(components::section_label(
            &format!("Overdue · {}", overdue.len()),
            theme,
        ));
        for (index, entry) in &overdue {
            body = body.child(task_row(*index, entry, theme, cx));
        }
    }
    body = body.child(components::section_label("Today", theme));
    if due_today.is_empty() {
        body = body.child(
            div()
                .py(theme.spacing.sm)
                .text_color(theme.palette.text_secondary)
                .child("Nothing due today."),
        );
    }
    for (index, entry) in &due_today {
        body = body.child(task_row(*index, entry, theme, cx));
    }
    body.into_any_element()
}

/// One task as a small box in a week column or the unscheduled row.
fn task_box(
    index: usize,
    entry: &Entry,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let entry = entry.clone();
    div()
        .id(("screen-box", index as u64))
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(div().child(entry.task.title().to_string()))
        .child(
            div()
                .w_full()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .truncate()
                .child(entry.project_name.clone()),
        )
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_screen_task(entry.clone(), cx)
            }),
        )
        .into_any_element()
}

fn week(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let today = DueDate::today_utc().day_number();
    let open: Vec<(usize, &Entry)> = view
        .screen_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.task.status() != looji_domain::Status::Done)
        .collect();

    let mut days = div().flex().flex_row().items_start().gap(theme.spacing.sm);
    for offset in 0..7 {
        let day = today + offset;
        let tasks: Vec<_> = open
            .iter()
            .copied()
            .filter(|(_, entry)| due_day(entry) == Some(day))
            .collect();
        let mut column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(theme.spacing.xs)
            .p(theme.spacing.xs)
            .rounded(theme.radius.lg)
            .bg(theme.palette.surface_raised)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px(theme.spacing.xs)
                    .child(div().font_weight(FontWeight::MEDIUM).child(short_date(day)))
                    .child(
                        div()
                            .text_size(theme.typography.caption)
                            .text_color(theme.palette.text_secondary)
                            .child(tasks.len().to_string()),
                    ),
            );
        if tasks.is_empty() {
            column = column.child(
                div()
                    .px(theme.spacing.xs)
                    .py(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_disabled)
                    .child("Nothing due"),
            );
        }
        for (index, entry) in &tasks {
            column = column.child(task_box(*index, entry, theme, cx));
        }
        days = days.child(column);
    }

    let unscheduled: Vec<_> = open
        .iter()
        .copied()
        .filter(|(_, entry)| due_day(entry).is_none())
        .collect();
    let mut loose = div().flex().flex_row().flex_wrap().gap(theme.spacing.sm);
    for (index, entry) in &unscheduled {
        // Each card gets a fixed width so the wrapping tray lays them out in even columns.
        loose = loose.child(div().w(px(180.)).child(task_box(*index, entry, theme, cx)));
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .id("screen-body")
        .overflow_y_scroll()
        .px(theme.spacing.lg)
        .pb(theme.spacing.lg)
        .child(days)
        .child(components::section_label(
            &format!("Unscheduled · {}", unscheduled.len()),
            theme,
        ))
        .child(loose)
        .into_any_element()
}
