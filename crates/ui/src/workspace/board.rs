//! The board: one column per status, each with a header showing its count and a scrollable body.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{prelude::*, *};
#[cfg(debug_assertions)]
use looji_domain::{Position, TaskRepository, Timestamp};
use looji_domain::{Project, Status, StatusColumn, Task, TaskId, status_columns};

use super::WorkspaceView;
use super::card::{CalendarDate, CardDensity, CardView, task_card};
use super::drag::{CardSlot, ColumnSlot};
#[cfg(debug_assertions)]
use super::dragging::Exit;
use super::dragging::{self, BoardLayout, DragSession};
use crate::theme::Theme;

const HEADER_HEIGHT: Pixels = px(36.);
const DROP_ZONE_HEIGHT: Pixels = px(96.);

/// Renders the selected project's board. Counts and cards come from one grouping of `tasks`,
/// so a header can't disagree with the cards under it.
pub(super) fn render(
    project: &Project,
    tasks: &[Task],
    drag: &DragSession,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let columns = status_columns(tasks, &project.visible_statuses());
    let today = CalendarDate::utc_today();
    let layout = drag.layout.clone();
    let dragged = drag.dragged();
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
                .on_children_prepainted({
                    let layout = layout.clone();
                    let statuses: Vec<Status> =
                        columns.iter().map(|column| column.status).collect();
                    move |bounds, _, _| {
                        let columns = statuses
                            .iter()
                            .zip(bounds)
                            .map(|(&status, bounds)| ColumnSlot {
                                status,
                                bounds: dragging::rect(bounds),
                            })
                            .collect();
                        layout.borrow_mut().set_columns(columns);
                    }
                })
                .children(columns.iter().enumerate().map(|(index, column)| {
                    column_view(index, column, theme, today, &layout, dragged, cx)
                })),
        );
    #[cfg(debug_assertions)]
    let root = root.child(debug_controls(theme, cx, drag.last_exit()));
    root
}

fn column_view(
    index: usize,
    column: &StatusColumn<'_>,
    theme: &Theme,
    today: CalendarDate,
    layout: &Rc<RefCell<BoardLayout>>,
    dragged: Option<TaskId>,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement + use<> {
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
        .child(body(index, column, theme, today, layout, dragged, cx))
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
    layout: &Rc<RefCell<BoardLayout>>,
    dragged: Option<TaskId>,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement + use<> {
    let content = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs);

    // The cards' bounds are recorded for the drop target. An empty column's child is the drop
    // zone, which pairs with no ids, so the column ends up with no cards.
    let ids: Vec<TaskId> = column.tasks.iter().map(|task| task.id()).collect();
    let status = column.status;
    let layout = layout.clone();
    let content = if column.tasks.is_empty() {
        content.child(drop_zone(index, theme))
    } else {
        content.children(
            column
                .tasks
                .iter()
                .map(|task| card_slot(task, today, theme, dragged, cx)),
        )
    };
    content
        .on_children_prepainted(move |bounds, _, _| {
            let cards = ids
                .iter()
                .copied()
                .zip(bounds)
                .map(|(task, bounds)| CardSlot {
                    task,
                    status,
                    bounds: dragging::rect(bounds),
                })
                .collect();
            layout.borrow_mut().set_cards(status, cards);
        })
        .id(("column-body", index as u64))
        .overflow_y_scroll()
}

/// A card with a press handler. The card under a drag is dimmed where it sits, so the column
/// doesn't reflow.
fn card_slot(
    task: &Task,
    today: CalendarDate,
    theme: &Theme,
    dragged: Option<TaskId>,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement + use<> {
    let id = task.id();
    div()
        .when(dragged == Some(id), |slot| slot.opacity(0.4))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.card_pressed(id, event, window, cx)
            }),
        )
        .child(task_card(
            card_view(task),
            CardDensity::Compact,
            today,
            theme,
        ))
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

pub(super) fn card_view(task: &Task) -> CardView<'_> {
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
pub(super) fn debug_controls(
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
    last_exit: Option<Exit>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(theme.spacing.sm)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(exit_text(last_exit))
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

/// What the last drag did, for checking drops and cancels by hand.
#[cfg(debug_assertions)]
fn exit_text(exit: Option<Exit>) -> String {
    match exit {
        None => String::new(),
        Some(Exit::Dropped(result)) => format!(
            "Last drop: {} at index {}",
            status_label(result.status),
            result.index
        ),
        Some(Exit::Cancelled(reason)) => format!("Last drag cancelled: {reason:?}"),
    }
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
