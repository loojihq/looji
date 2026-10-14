//! The board: one column per status, each with a header showing its count and a scrollable body.

use std::time::Duration;

use gpui::{prelude::*, *};
#[cfg(debug_assertions)]
use looji_domain::{Position, TaskRepository};
use looji_domain::{Project, Status, StatusColumn, Task, TaskId, status_columns};

use super::WorkspaceView;
use super::card::{AssigneeMark, CalendarDate, CardDensity, CardView, task_card};
#[cfg(debug_assertions)]
use super::drag::Exit;
use super::dragging::{DragSession, record_columns};
use super::landing::{Slot, column_sequence, stack_tops};
use super::members;
use super::search;
use super::task_edit::TagEdit;
use super::task_editors;
use super::task_form::Mode;
use crate::theme::Theme;

const HEADER_HEIGHT: Pixels = px(36.);
const DROP_ZONE_HEIGHT: Pixels = px(96.);
/// The placeholder's height in an empty column, where there's no dragged card to measure.
const EMPTY_SLOT_HEIGHT: f32 = 56.;
/// How long siblings take to slide out of the way of the placeholder.
const SHIFT_DURATION: Duration = Duration::from_millis(150);

/// Renders the selected project's board. Counts and cards come from one grouping of `tasks`,
/// so a header can't disagree with the cards under it.
pub(super) fn render(
    project: &Project,
    tasks: &[Task],
    cards: &CardContext,
    drag: &DragSession,
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
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.sm)
                        .child(new_task_button(theme, cx))
                        .child(column_toggle(project, theme, cx)),
                ),
        )
        .when_some(drag.keyboard_notice(), |header, text| {
            header.child(
                div()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(text.to_string()),
            )
        })
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
                .on_children_prepainted(record_columns(
                    &drag.layout,
                    columns.iter().map(|column| column.status).collect(),
                ))
                .children(columns.iter().enumerate().map(|(index, column)| {
                    column_view(index, column, cards, theme, today, drag, cx)
                })),
        );
    #[cfg(debug_assertions)]
    let root = root.child(debug_controls(theme, cx, drag.last_exit()));
    root
}

fn column_view(
    index: usize,
    column: &StatusColumn<'_>,
    cards: &CardContext,
    theme: &Theme,
    today: CalendarDate,
    drag: &DragSession,
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
        .child(body(index, column, cards, theme, today, drag, cx))
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
///
/// The items come from `column_sequence`, the same function the drop uses. While a card is being
/// dragged over this column, the placeholder sits at the landing index, and the card under the
/// drag leaves its slot if the drag is in this column.
fn body(
    index: usize,
    column: &StatusColumn<'_>,
    cards: &CardContext,
    theme: &Theme,
    today: CalendarDate,
    drag: &DragSession,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement + use<> {
    let session = drag.session();
    let status = column.status;
    let dragged = drag.dragged();
    let source = session.source();
    let source_task = source
        .filter(|source| source.status == status)
        .map(|s| s.task);
    let ids: Vec<TaskId> = column.tasks.iter().map(|task| task.id()).collect();

    let now = session
        .landing()
        .filter(|landing| landing.status == status)
        .map(|landing| landing.index);
    let before = session
        .previous()
        .filter(|landing| landing.status == status)
        .map(|landing| landing.index);
    let sequence = column_sequence(&ids, source_task.filter(|_| now.is_some()), now);
    let earlier = column_sequence(&ids, source_task.filter(|_| before.is_some()), before);

    // The placeholder is as tall as the card it stands in for, or a fixed height in an empty column.
    let others = ids.iter().filter(|id| Some(**id) != source_task).count();
    let placeholder_height = match source {
        Some(source) if others > 0 => source.height,
        _ => EMPTY_SLOT_HEIGHT,
    };

    let gap = f32::from(theme.spacing.xs);
    let (tops, earlier_tops) = {
        let layout = drag.layout.borrow();
        let heights = |sequence: &[Slot]| -> Vec<f32> {
            sequence
                .iter()
                .map(|slot| match slot {
                    Slot::Task(task) => layout
                        .card(*task)
                        .map_or(placeholder_height, |card| card.bounds.height),
                    Slot::Placeholder => placeholder_height,
                })
                .collect()
        };
        (
            stack_tops(&heights(&sequence), gap),
            stack_tops(&heights(&earlier), gap),
        )
    };

    // Each item slides from where it was in the last landing to where it is now.
    let key_base = format!(
        "shift-{}-{}",
        status.as_str(),
        now.map_or("none".to_string(), |index| index.to_string())
    );
    let content = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs);

    let content = if sequence.is_empty() {
        content.child(drop_zone(index, !cards.terms.is_empty(), theme))
    } else {
        let items = sequence.iter().enumerate().map(|(position, slot)| {
            let offset = earlier
                .iter()
                .position(|earlier_slot| earlier_slot == slot)
                .map_or(0.0, |earlier_position| {
                    earlier_tops[earlier_position] - tops[position]
                });
            let key = SharedString::from(format!("{key_base}-{position}"));
            match slot {
                Slot::Task(id) => {
                    let task = column
                        .tasks
                        .iter()
                        .find(|task| task.id() == *id)
                        .expect("the sequence only holds this column's tasks");
                    let focus = drag.focus_handle(*id, cx);
                    shifted(
                        card_slot(task, cards, today, theme, dragged, &focus, cx),
                        key,
                        offset,
                    )
                }
                Slot::Placeholder => shifted(placeholder(placeholder_height, theme), key, offset),
            }
        });
        content.children(items)
    };

    let slots: Vec<Option<TaskId>> = sequence
        .iter()
        .map(|slot| match slot {
            Slot::Task(task) => Some(*task),
            Slot::Placeholder => None,
        })
        .collect();
    drag.layout.borrow_mut().set_items(status, slots);
    content
        .id(("column-body", index as u64))
        .overflow_y_scroll()
        .track_scroll(&drag.scroll_handle(status))
}

/// Glides an item the given distance from its place to its place now, so siblings slide to make
/// room for the placeholder instead of jumping.
///
/// The item sits inside a plain wrapper, and only the item moves. The wrapper keeps its place in
/// the layout, so the paint callbacks still record the resting positions the drop reads.
fn shifted<E>(element: E, key: SharedString, offset: f32) -> Div
where
    E: IntoElement + Styled + 'static,
{
    let item = if offset.abs() < 0.5 {
        element.into_any_element()
    } else {
        element
            .with_animation(
                key,
                Animation::new(SHIFT_DURATION).with_easing(ease_out_quint()),
                move |element, delta| element.relative().top(px(offset * (1.0 - delta))),
            )
            .into_any_element()
    };
    div().child(item)
}

/// The slot the dragged card would land in, shown while the drag is over this column.
fn placeholder(height: f32, theme: &Theme) -> Div {
    div()
        .flex_shrink_0()
        .h(px(height))
        .rounded(theme.radius.md)
        .border_1()
        .border_dashed()
        .border_color(theme.palette.border_control)
}

/// A card with a press handler. The card under a drag is dimmed where it sits, unless the
/// placeholder has taken its place.
fn card_slot(
    task: &Task,
    cards: &CardContext,
    today: CalendarDate,
    theme: &Theme,
    dragged: Option<TaskId>,
    focus: &FocusHandle,
    cx: &mut Context<WorkspaceView>,
) -> Stateful<Div> {
    let id = task.id();
    div()
        .id(("card", id.as_uuid().as_u128() as u64))
        .track_focus(focus)
        .tab_index(0)
        .rounded(theme.radius.sm)
        .border_1()
        .border_color(theme.palette.surface_raised)
        .focus(|style| style.border_color(theme.palette.accent))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            this.card_key(id, event, window, cx)
        }))
        .when(dragged == Some(id), |slot| slot.opacity(0.4))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.card_pressed(id, event, window, cx)
            }),
        )
        .child(task_card(
            card_view(task, cards),
            CardDensity::Compact,
            today,
            theme,
            Some(card_editor(task, cards, theme, cx)),
        ))
}

/// The foot of a card: its editor while it's open, otherwise an "Edit" link that opens it. The
/// editor's presses are contained, so a press here never starts a drag of the card.
fn card_editor(
    task: &Task,
    cards: &CardContext,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let id = task.id();
    if cards.editing != Some(id) {
        return div()
            .flex()
            .flex_row()
            .justify_end()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id(("card-edit", 0_u64))
                    .px(theme.spacing.xs)
                    .cursor_pointer()
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child("Edit")
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_card_editor(id, window, cx)
                    })),
            )
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(task_editors::priority_picker(
            "card-priority",
            task,
            theme,
            cx,
        ))
        .child(task_editors::tag_editor(
            "card-tags",
            task,
            cards.tag_edit.as_ref(),
            theme,
            cx,
        ))
        .child(
            div().flex().flex_row().justify_end().child(
                div()
                    .id(("card-done", 0_u64))
                    .px(theme.spacing.sm)
                    .cursor_pointer()
                    .text_size(theme.typography.caption)
                    .child("Done")
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.close_card_editor(cx)),
                    ),
            ),
        )
        .into_any_element()
}

/// What the cards need from the view beyond their own task: the team, and the card editor that's
/// open, if any.
pub(super) struct CardContext {
    pub members: Vec<members::Entry>,
    pub editing: Option<TaskId>,
    pub tag_edit: Option<TagEdit>,
    /// The search terms in use. Empty when nothing is being searched.
    pub terms: Vec<String>,
}

/// What the cards draw from the view, built fresh for each render so it can't go stale.
pub(super) fn card_context(view: &WorkspaceView) -> CardContext {
    CardContext {
        members: members::entries(&view.members),
        editing: view.card_editing,
        tag_edit: view.tag_edit.clone(),
        terms: search::terms(&view.search_query),
    }
}

/// The empty-column target. It keeps its height and has a stable ID, so drag can find it later.
/// While a search is running, an empty column says so, since its cards may be hidden.
fn drop_zone(index: usize, searching: bool, theme: &Theme) -> impl IntoElement {
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
        .child(if searching { "No matches" } else { "No tasks" })
}

/// The card for a task. A task whose assignee isn't in `members` shows as unassigned, since it can't
/// be named.
pub(super) fn card_view<'a>(task: &'a Task, cards: &CardContext) -> CardView<'a> {
    let members = &cards.members;
    CardView {
        id: task.id(),
        title: task.title(),
        priority: task.priority().map(|priority| match priority {
            looji_domain::Priority::Low => super::card::Priority::Low,
            looji_domain::Priority::Medium => super::card::Priority::Medium,
            looji_domain::Priority::High => super::card::Priority::High,
        }),
        assignee: task
            .assignee_id()
            .and_then(|id| members.iter().find(|entry| entry.id == id))
            .map(|entry| AssigneeMark {
                initials: entry.initials.clone(),
                label: entry.label.clone(),
                colour: entry.colour,
            }),
        estimate_minutes: task.estimate().map(|estimate| estimate.minutes()),
        due: task
            .due_on()
            .and_then(|due| CalendarDate::new(due.year(), due.month(), due.day())),
        tags: task.tags().to_vec(),
        search: (!cards.terms.is_empty()).then(|| search::hits(task, &cards.terms)),
    }
}

pub(super) fn status_label(status: Status) -> &'static str {
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
        Some(Exit::Unchanged) => "Last drop: same place, nothing saved".to_string(),
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
        // After the column's last task, so the sample lands at the end.
        let last = self
            .board
            .iter()
            .rev()
            .find(|other| other.status() == status)
            .map(|other| other.position().as_str().to_string());
        let position =
            Position::new(looji_domain::after(last.as_deref()).expect("stored keys are valid"))
                .expect("keys are never empty");
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
                self.close_panel_if_gone(cx);
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

/// The button that starts a new task in the first shown column.
fn new_task_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("new-task")
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
        .child("+ New task")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.open_task_dialog(Mode::Create, window, cx)
        }))
}
