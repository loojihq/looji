//! The roadmap: tasks as bars on a calendar, in rows grouped by assignee or by tag, with a tray for the
//! tasks that have no dates. The dates and the layout are in `roadmap_layout.rs`; this file draws them.
//!
//! Bars are drawn from the stored start and due dates. Nothing is calculated, so a date a user changes
//! moves its bar at the next render. The board's filters and search apply, through the same function the
//! board uses, so the roadmap shows the same tasks the board does.

use gpui::{prelude::*, *};
use looji_domain::{DueDate, Task};

use super::board::status_label;
use super::card::tooltip;
use super::members;
use super::roadmap_layout::{
    GroupKey, Grouping, Scale, Schedule, Window, bar_for, group_counts, is_weekend, month_spans,
    schedule, weekday,
};
use super::{WorkspaceView, board_search};
use crate::theme::Theme;

/// The width of the column of row labels, in pixels.
const LABEL_WIDTH: f32 = 220.;
const ROW_HEIGHT: f32 = 30.;
const HEADER_HEIGHT: f32 = 22.;

/// What the roadmap is showing: the scale, the grouping, and the day it's centred on. `None` means today,
/// so the default follows the calendar.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RoadmapState {
    pub scale: Scale,
    pub grouping: Grouping,
    pub anchor: Option<i64>,
}

impl RoadmapState {
    pub fn anchor_day(&self) -> i64 {
        self.anchor
            .unwrap_or_else(|| DueDate::today_utc().day_number())
    }

    /// How far Earlier and Later move the window: two weeks on the week scale, two months on the month scale.
    fn step(&self) -> i64 {
        match self.scale {
            Scale::Week => 14,
            Scale::Month => 60,
        }
    }
}

impl WorkspaceView {
    pub(super) fn set_roadmap_scale(&mut self, scale: Scale, cx: &mut Context<Self>) {
        self.roadmap.scale = scale;
        cx.notify();
    }

    pub(super) fn set_roadmap_grouping(&mut self, grouping: Grouping, cx: &mut Context<Self>) {
        self.roadmap.grouping = grouping;
        cx.notify();
    }

    pub(super) fn shift_roadmap(&mut self, days: i64, cx: &mut Context<Self>) {
        let anchor = self.roadmap.anchor_day();
        self.roadmap.anchor = Some(anchor + days);
        cx.notify();
    }

    pub(super) fn roadmap_today(&mut self, cx: &mut Context<Self>) {
        self.roadmap.anchor = None;
        cx.notify();
    }
}

/// One group of rows: the tasks in it, split into those on the timeline and those in the tray.
struct Group<'a> {
    key: GroupKey,
    scheduled: Vec<(&'a Task, Schedule)>,
}

/// The roadmap body: the toolbar, the calendar and rows, and the tray. Filters and search come from the
/// board, so the same tasks are shown.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let state = view.roadmap;
    let window = Window::around(state.scale, state.anchor_day());
    let shown = board_search::shown_tasks(view);
    let member_order: Vec<_> = view.members.iter().map(|member| member.id()).collect();
    let entries = members::entries(&view.members);

    let (groups, tray) = partition(&shown, state.grouping, &member_order);

    let mut body = div()
        .id("roadmap-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_scroll()
        .child(header(window, state.scale, theme));
    let overlay_top = HEADER_HEIGHT * 2.;
    let mut rows = div()
        .flex()
        .flex_col()
        .relative()
        .w(px(LABEL_WIDTH) + px(window.width(state.scale)));
    let mut has_rows = false;
    for group in &groups {
        has_rows = true;
        rows = rows.child(group_row(group, &entries, theme));
        for (task, schedule) in &group.scheduled {
            rows = rows.child(task_row(task, *schedule, window, state.scale, theme, cx));
        }
    }
    body = body.child(
        div()
            .relative()
            .child(columns_overlay(window, state.scale, theme, overlay_top))
            .child(rows),
    );
    if !has_rows {
        body = body.child(
            div()
                .px(theme.spacing.lg)
                .py(theme.spacing.md)
                .text_color(theme.palette.text_secondary)
                .child("Nothing on the timeline in this view. Give a task a start and a due date to see it here."),
        );
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(toolbar(state, theme, cx))
        .child(body)
        .child(tray_view(&tray, theme, cx))
        .into_any_element()
}

/// Splits the tasks into groups, with the timeline's schedule for each, and the tray. A task with a date
/// problem stays on the timeline when both its dates are set, flagged as inverted. Only a task with fewer
/// than two dates goes in the tray.
fn partition<'a>(
    tasks: &'a [Task],
    grouping: Grouping,
    member_order: &[looji_domain::TeamMemberId],
) -> (Vec<Group<'a>>, Vec<(&'a Task, Schedule)>) {
    let mut tray = Vec::new();
    let mut groups: Vec<Group<'a>> = group_counts(tasks, grouping, member_order)
        .into_iter()
        .map(|count| Group {
            key: count.key,
            scheduled: Vec::new(),
        })
        .collect();
    for task in tasks {
        let placed = schedule(task.start_on(), task.due_on());
        match placed {
            Schedule::Unscheduled(_) => tray.push((task, placed)),
            _ => {
                let key = GroupKey::of(task, grouping);
                if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                    group.scheduled.push((task, placed));
                }
            }
        }
    }
    (groups, tray)
}

fn toolbar(state: RoadmapState, theme: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
    let scale_button = |label: &'static str, scale: Scale, id: u64| {
        segment(label, state.scale == scale, theme)
            .id(("roadmap-scale", id))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.set_roadmap_scale(scale, cx)),
            )
    };
    let grouping_button = |label: &'static str, grouping: Grouping, id: u64| {
        segment(label, state.grouping == grouping, theme)
            .id(("roadmap-grouping", id))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_roadmap_grouping(grouping, cx)
            }))
    };
    let nav = |label: &'static str, id: u64| segment(label, false, theme).id(("roadmap-nav", id));
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.lg)
        .py(theme.spacing.xs)
        .border_b_1()
        .border_color(theme.palette.border)
        .child(scale_button("Week", Scale::Week, 0))
        .child(scale_button("Month", Scale::Month, 1))
        .child(div().w(px(12.)))
        .child(grouping_button("By assignee", Grouping::Assignee, 0))
        .child(grouping_button("By tag", Grouping::Tag, 1))
        .child(div().w(px(12.)))
        .child(
            nav("Earlier", 0).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let step = -this.roadmap.step();
                this.shift_roadmap(step, cx)
            })),
        )
        .child(
            nav("Today", 1)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.roadmap_today(cx))),
        )
        .child(
            nav("Later", 2).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let step = this.roadmap.step();
                this.shift_roadmap(step, cx)
            })),
        )
        .into_any_element()
}

fn segment(label: &'static str, selected: bool, theme: &Theme) -> Div {
    div()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.caption)
        .cursor_pointer()
        .when(selected, |button| button.bg(theme.palette.surface_selected))
        .when(!selected, |button| {
            button.hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(label)
}

/// The calendar header: months above, and the days below. On the week scale every day is labelled. On the
/// month scale each Monday is.
fn header(window: Window, scale: Scale, theme: &Theme) -> impl IntoElement {
    let width = window.width(scale);
    let day_width = scale.day_width();
    let mut months = div()
        .relative()
        .h(px(HEADER_HEIGHT))
        .w(px(width))
        .ml(px(LABEL_WIDTH));
    for span in month_spans(window) {
        months = months.child(
            div()
                .absolute()
                .top_0()
                .left(px(span.first_column as f32 * day_width))
                .w(px(span.columns as f32 * day_width))
                .h(px(HEADER_HEIGHT))
                .overflow_hidden()
                .px(theme.spacing.xs)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("{} {}", month_name(span.month), span.year)),
        );
    }

    let mut days = div()
        .relative()
        .h(px(HEADER_HEIGHT))
        .w(px(width))
        .ml(px(LABEL_WIDTH))
        .border_b_1()
        .border_color(theme.palette.border);
    for offset in 0..window.count {
        let label = match scale {
            Scale::Week => Some(
                DueDate::from_day_number(window.first + offset)
                    .day()
                    .to_string(),
            ),
            Scale::Month => (weekday(window.first + offset) == 0).then(|| {
                DueDate::from_day_number(window.first + offset)
                    .day()
                    .to_string()
            }),
        };
        if let Some(label) = label {
            days = days.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(offset as f32 * day_width))
                    .w(px(day_width))
                    .h(px(HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(label),
            );
        }
    }
    div().flex().flex_col().child(months).child(days)
}

/// Weekend shading and today's line, drawn behind the rows across their full height.
fn columns_overlay(window: Window, scale: Scale, theme: &Theme, top: f32) -> impl IntoElement {
    let day_width = scale.day_width();
    let mut overlay = div()
        .absolute()
        .top(px(top))
        .bottom_0()
        .left(px(LABEL_WIDTH))
        .w(px(window.width(scale)));
    for offset in 0..window.count {
        if is_weekend(window.first + offset) {
            overlay = overlay.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(offset as f32 * day_width))
                    .w(px(day_width))
                    .bg(theme.palette.surface_hover),
            );
        }
    }
    let today = DueDate::today_utc().day_number();
    if window.contains(today) {
        overlay = overlay.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px((today - window.first) as f32 * day_width
                    + day_width / 2.
                    - 0.5))
                .w(px(1.))
                .bg(theme.palette.accent),
        );
    }
    overlay
}

fn group_row(group: &Group<'_>, entries: &[members::Entry], theme: &Theme) -> impl IntoElement {
    let label = match &group.key {
        GroupKey::Member(id) => entries
            .iter()
            .find(|entry| entry.id == *id)
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| "Removed member".to_string()),
        GroupKey::Unassigned => "Unassigned".to_string(),
        GroupKey::Tag(tag) => tag.clone(),
        GroupKey::NoTag => "No tag".to_string(),
    };
    let inverted = group
        .scheduled
        .iter()
        .filter(|(_, placed)| matches!(placed, Schedule::Inverted { .. }))
        .count();
    let mut summary = format!("{} scheduled", group.scheduled.len());
    if inverted > 0 {
        summary.push_str(&format!(" · {inverted} flagged"));
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(ROW_HEIGHT))
        .px(theme.spacing.lg)
        .bg(theme.palette.surface_raised)
        .border_b_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.caption)
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_shrink_0()
                .truncate()
                .child(label),
        )
        .child(
            div()
                .text_color(theme.palette.text_secondary)
                .child(summary),
        )
}

fn task_row(
    task: &Task,
    placed: Schedule,
    window: Window,
    scale: Scale,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = task.id();
    let label = div()
        .w(px(LABEL_WIDTH))
        .flex_shrink_0()
        .truncate()
        .px(theme.spacing.lg)
        .child(task.title().to_string());
    let timeline = div()
        .relative()
        .h(px(ROW_HEIGHT))
        .w(px(window.width(scale)));
    let (first, last, inverted) = match placed {
        Schedule::Planned { first, last } => (first, last, false),
        Schedule::Inverted { first, last } => (first, last, true),
        Schedule::Unscheduled(_) => return div().h(px(ROW_HEIGHT)),
    };
    let timeline = match bar_for(first, last, inverted, window, scale) {
        Some(bar) => timeline.child(bar_view(task, bar, id, theme, cx)),
        None => timeline.child(
            div()
                .px(theme.spacing.xs)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_disabled)
                .child("outside this view"),
        ),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(theme.palette.border)
        .child(label)
        .child(timeline)
}

/// A task's bar. An inverted one is drawn as a dashed, tinted bar in the danger colour, with the words
/// "Due before start" on it, so the flag doesn't depend on colour alone. The tooltip gives both dates.
fn bar_view(
    task: &Task,
    bar: super::roadmap_layout::Bar,
    id: looji_domain::TaskId,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let detail = match (task.start_on(), task.due_on()) {
        (Some(start), Some(due)) => format!("Start {start} · Due {due}"),
        _ => String::new(),
    };
    let mut view = div()
        .id(("roadmap-bar", id.as_uuid().as_u128() as u64))
        .absolute()
        .top(px(5.))
        .h(px(ROW_HEIGHT - 10.))
        .left(px(bar.left))
        .w(px(bar.width))
        .min_w_0()
        .overflow_hidden()
        .rounded(theme.radius.sm)
        .px(theme.spacing.xs)
        .flex()
        .items_center()
        .text_size(theme.typography.caption)
        .cursor_pointer()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx)));
    if bar.inverted {
        view = view
            .bg(theme.palette.danger.opacity(0.18))
            .border_1()
            .border_dashed()
            .border_color(theme.palette.danger)
            .text_color(theme.palette.danger)
            .child(
                div()
                    .truncate()
                    .child(format!("⚠ Due before start · {}", task.title())),
            );
    } else {
        view = view
            .bg(theme.palette.accent)
            .text_color(theme.palette.on_accent)
            .child(div().truncate().child(task.title().to_string()));
    }
    let status = status_label(task.status());
    view.tooltip(tooltip(format!("{} · {status} · {detail}", task.title())))
}

/// The unscheduled tray: every task with fewer than two dates, with why it's there. Clicking one opens it.
fn tray_view(
    tray: &[(&Task, Schedule)],
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let mut chips = div().flex().flex_row().flex_wrap().gap(theme.spacing.xs);
    for (index, (task, placed)) in tray.iter().enumerate() {
        let reason = match placed {
            Schedule::Unscheduled(reason) => reason.label(),
            _ => "",
        };
        let id = task.id();
        chips = chips.child(
            div()
                .id(("roadmap-tray", index as u64))
                .flex()
                .flex_row()
                .gap(theme.spacing.xs)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .border_1()
                .border_dashed()
                .border_color(theme.palette.border_control)
                .text_size(theme.typography.caption)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child(task.title().to_string())
                .child(
                    div()
                        .text_color(theme.palette.text_secondary)
                        .child(format!("· {reason}")),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx))),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .px(theme.spacing.lg)
        .py(theme.spacing.sm)
        .max_h(px(140.))
        .border_t_1()
        .border_color(theme.palette.border)
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("Unscheduled ({})", tray.len())),
        )
        .child(if tray.is_empty() {
            div()
                .text_color(theme.palette.text_secondary)
                .child("Every task in this view has its dates.")
                .into_any_element()
        } else {
            chips.into_any_element()
        })
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}
