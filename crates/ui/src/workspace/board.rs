//! The board: one column per status, each with a header showing its count and a scrollable body.

use gpui::{prelude::*, *};
#[cfg(debug_assertions)]
use looji_domain::{Position, TaskRepository, Timestamp};
use looji_domain::{Project, Status, StatusColumn, Task, status_columns};

use super::WorkspaceView;
use super::card::{CalendarDate, CardDensity, CardView, task_card};
use crate::theme::Theme;

const HEADER_HEIGHT: Pixels = px(36.);
const DROP_ZONE_HEIGHT: Pixels = px(96.);

/// Renders the selected project's board. Counts and cards come from one grouping of `tasks`,
/// so a header can't disagree with the cards under it.
pub(super) fn render(
    project: &Project,
    tasks: &[Task],
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let columns = status_columns(tasks, &project.visible_statuses());
    let today = CalendarDate::utc_today();
    #[cfg(not(debug_assertions))]
    let _ = cx;
    let root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .p(theme.spacing.lg)
        .gap(theme.spacing.md)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(theme.spacing.sm)
                .child(
                    div()
                        .text_size(theme.typography.heading)
                        .child(project.name().to_string()),
                )
                .child(column_toggle(project, theme, cx)),
        )
        .when_some(project.description(), |header, text| {
            header.child(
                div()
                    .text_size(theme.typography.body)
                    .text_color(theme.palette.text_secondary)
                    .child(text.to_string()),
            )
        })
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .gap(theme.spacing.sm)
                .children(
                    columns
                        .iter()
                        .enumerate()
                        .map(|(index, column)| column_view(index, column, theme, today)),
                ),
        );
    #[cfg(debug_assertions)]
    let root = root.child(debug_controls(theme, cx));
    root
}

fn column_view(
    index: usize,
    column: &StatusColumn<'_>,
    theme: &Theme,
    today: CalendarDate,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .rounded(theme.radius.md)
        .bg(theme.palette.surface_raised)
        .border_1()
        .border_color(theme.palette.border)
        .child(header(column, theme))
        .child(body(index, column, theme, today))
}

fn header(column: &StatusColumn<'_>, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .h(HEADER_HEIGHT)
        .px(theme.spacing.sm)
        .text_size(theme.typography.body)
        .child(status_label(column.status))
        .child(
            div()
                .px(theme.spacing.sm)
                .rounded(theme.radius.sm)
                .bg(theme.palette.surface_selected)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(column.count().to_string()),
        )
}

/// The scrollable card area. Only this part scrolls, so the header stays put.
fn body(
    index: usize,
    column: &StatusColumn<'_>,
    theme: &Theme,
    today: CalendarDate,
) -> impl IntoElement {
    let body = div()
        .id(("column-body", index as u64))
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs);

    if column.tasks.is_empty() {
        body.child(drop_zone(index, theme))
    } else {
        body.children(
            column
                .tasks
                .iter()
                .map(|task| task_card(card_view(task), CardDensity::Compact, today, theme)),
        )
    }
}

/// The empty-column target. It keeps its height and has a stable ID, so drag can find it later.
fn drop_zone(index: usize, theme: &Theme) -> impl IntoElement {
    div()
        .id(("drop-zone", index as u64))
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(DROP_ZONE_HEIGHT)
        .rounded(theme.radius.md)
        .border_1()
        .border_dashed()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child("No tasks")
}

fn card_view(task: &Task) -> CardView<'_> {
    CardView {
        id: task.id(),
        title: task.title(),
        priority: None,
        assignee: None,
        estimate_minutes: task.estimate().map(|estimate| estimate.minutes()),
        due: None,
    }
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Backlog => "Backlog",
        Status::Todo => "To do",
        Status::InProgress => "In progress",
        Status::InReview => "In review",
        Status::Done => "Done",
    }
}

/// Temporary controls for creating and deleting tasks until the create dialog and the delete
/// flow exist. Compiled only in debug builds.
#[cfg(debug_assertions)]
pub(super) fn debug_controls(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(theme.spacing.sm)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(
            div()
                .id("debug-add-task")
                .cursor_pointer()
                .hover(|style| style.text_color(theme.palette.text_primary))
                .child("+ sample task")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_sample_task(cx))),
        )
        .child(
            div()
                .id("debug-delete-task")
                .cursor_pointer()
                .hover(|style| style.text_color(theme.palette.text_primary))
                .child("− last task")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.delete_last_task(cx))),
        )
}

impl WorkspaceView {
    /// Creates a task in the selected project and adds it to the board. Debug builds only.
    #[cfg(debug_assertions)]
    pub(super) fn add_sample_task(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        let visible = self
            .projects
            .active()
            .map(Project::visible_statuses)
            .unwrap_or_default();
        if visible.is_empty() {
            return;
        }
        let status = visible[self.board.len() % visible.len()];
        let title = format!("Sample task {}", self.board.len() + 1);
        // Zero-padded timestamps sort in creation order.
        let position = Position::new(format!("{:020}", Timestamp::now().unix_millis()))
            .expect("timestamps are never empty");
        let task = Task::new(project, &title, status, position).expect("sample titles are valid");
        match TaskRepository::save(super::store(cx), &task) {
            Ok(()) => self.board.push(task),
            Err(err) => self.status = Some(format!("Could not create the task: {err}")),
        }
        cx.notify();
    }

    /// Deletes the last task on the board. Debug builds only.
    #[cfg(debug_assertions)]
    pub(super) fn delete_last_task(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.board.last().map(Task::id) else {
            return;
        };
        match TaskRepository::delete(super::store(cx), id) {
            Ok(()) => {
                self.board.pop();
            }
            Err(err) => self.status = Some(format!("Could not delete the task: {err}")),
        }
        cx.notify();
    }
}

/// The toggle for the optional In review column. Hiding a column that has tasks asks first.
fn column_toggle(
    project: &Project,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let shown = project.is_status_visible(Status::InReview);
    let label = if shown {
        "In review column: shown"
    } else {
        "In review column: hidden"
    };
    div()
        .id("column-toggle-in-review")
        .flex_shrink_0()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.request_column_visibility(Status::InReview, !shown, cx);
        }))
}
