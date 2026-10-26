//! The board: one column per status, each with a header showing its count and a scrollable body.

use std::time::Duration;

use gpui::{prelude::*, *};

use looji_domain::{Attachment, Project, Status, StatusColumn, Task, TaskId, status_columns};

use super::WorkspaceView;
use super::card::{
    AssigneeMark, CalendarDate, CardView, card_group, civil_from_days, format_due, format_estimate,
    task_card,
};
use super::dragging::{DragSession, record_columns};
use super::landing::{Slot, column_sequence, stack_tops};
use super::media;
use super::members;
use super::search;

use super::task_form::Mode;
use crate::components;
use crate::database::AppDatabase;
use crate::theme::Theme;

const HEADER_HEIGHT: Pixels = px(36.);
const DROP_ZONE_HEIGHT: Pixels = px(96.);
/// The placeholder's height in an empty column, where there's no dragged card to measure.
const EMPTY_SLOT_HEIGHT: f32 = 56.;
/// The height assumed for a card that has never been drawn, until it has been measured.
const ESTIMATED_CARD_HEIGHT: f32 = 80.;
/// How much of a column is drawn in full on the first frame, before its size is known.
const FIRST_FRAME_WINDOW: f32 = 1200.;
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
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .p(theme.spacing.lg)
        .gap(theme.spacing.md)
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
        )
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
        .rounded(theme.radius.lg)
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
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.palette.text_primary)
                .child(status_label(column.status)),
        )
        // The count sits on the right as plain text, as in the design, not in a pill.
        .child(
            div()
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
    let handle = drag.scroll_handle(status);
    // A card nobody has measured is assumed to be a typical card, or the dragged card's height during a drag.
    let unmeasured = if source.is_some() {
        placeholder_height
    } else {
        ESTIMATED_CARD_HEIGHT
    };
    let (tops, earlier_tops, heights_now) = {
        let mut layout = drag.layout.borrow_mut();
        // What the last frame laid out is measured now. A card drawn in full gives its true height, which is kept for
        // when it scrolls out of view and a spacer of that height takes its place.
        let previous: Vec<Option<TaskId>> = layout
            .items
            .iter()
            .find(|(column, _)| *column == status)
            .map(|(_, items)| items.clone())
            .unwrap_or_default();
        for (position, item) in previous.iter().enumerate() {
            if let Some(task) = item
                && layout
                    .real
                    .get(&status)
                    .is_some_and(|real| real.contains(task))
                && let Some(bounds) = handle.bounds_for_item(position)
            {
                layout.measured.insert(*task, f32::from(bounds.size.height));
            }
        }
        let heights = |sequence: &[Slot]| -> Vec<f32> {
            sequence
                .iter()
                .map(|slot| match slot {
                    Slot::Task(task) => layout.measured.get(task).copied().unwrap_or(unmeasured),
                    Slot::Placeholder => placeholder_height,
                })
                .collect()
        };
        let now_heights = heights(&sequence);
        (
            stack_tops(&now_heights, gap),
            stack_tops(&heights(&earlier), gap),
            now_heights,
        )
    };

    // Only the cards near the visible part of the column are drawn in full. The rest are spacers of the same height,
    // one per card, so every card keeps its place and its index in the column, which the drag reads. A screenful of
    // margin is drawn each side so a scroll doesn't show empty space before the next frame.
    let viewport = f32::from(handle.bounds().size.height);
    let scroll_top = (-f32::from(handle.offset().y)).max(0.0);
    let (window_start, window_end) = if viewport > 0.0 {
        (scroll_top - viewport, scroll_top + 2.0 * viewport)
    } else {
        (0.0, FIRST_FRAME_WINDOW)
    };
    let drawn_in_full: std::collections::HashSet<TaskId> = sequence
        .iter()
        .enumerate()
        .filter_map(|(position, slot)| match slot {
            Slot::Task(id) => {
                let (top, height) = (tops[position], heights_now[position]);
                let near = top + height >= window_start && top <= window_end;
                // The card being dragged or edited is always real, wherever it is.
                let pinned =
                    dragged == Some(*id) || source_task == Some(*id) || cards.expanded == Some(*id);
                (near || pinned).then_some(*id)
            }
            Slot::Placeholder => None,
        })
        .collect();
    drag.layout
        .borrow_mut()
        .real
        .insert(status, drawn_in_full.clone());

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
        // Both lookups are built once. Searching the column for each card would cost a pass over every card for every
        // card, on every frame, which is what made a large column slow.
        let earlier_position: std::collections::HashMap<Slot, usize> = earlier
            .iter()
            .enumerate()
            .map(|(index, slot)| (*slot, index))
            .collect();
        let by_id: std::collections::HashMap<TaskId, &Task> =
            column.tasks.iter().map(|task| (task.id(), *task)).collect();
        let items = sequence.iter().enumerate().map(|(position, slot)| {
            let offset = earlier_position.get(slot).map_or(0.0, |earlier_position| {
                earlier_tops[*earlier_position] - tops[position]
            });
            let key = SharedString::from(format!("{key_base}-{position}"));
            match slot {
                Slot::Task(id) if !drawn_in_full.contains(id) => {
                    // Off screen: a spacer as tall as the card, so the column's length and every card's index hold.
                    div().flex_shrink_0().h(px(heights_now[position]))
                }
                Slot::Task(id) => {
                    let task = by_id
                        .get(id)
                        .copied()
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
        // A scroll can fold the project header, so the window redraws on each one.
        .on_scroll(cx.listener(|_, _: &ScrollEvent, _, cx| cx.notify()))
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
    let group = card_group(id);
    div()
        .id(("card", id.as_uuid().as_u128() as u64))
        .group(group.clone())
        .relative()
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
            today,
            theme,
            (cards.expanded == Some(task.id()))
                .then(|| card_details(task, cards, today, theme, cx)),
        ))
        .child(
            div().absolute().top(px(6.)).right(px(6.)).child(
                components::icon_button(
                    ("card-edit", id.as_uuid().as_u128() as u64),
                    "edit",
                    theme,
                )
                .invisible()
                .group_hover(group, |style| style.visible())
                // The press stays with the pencil, so it never starts a drag of the card.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_task_dialog(Mode::Edit(id), window, cx)
                })),
            ),
        )
}

/// The most characters of a description the expanded card shows. A longer one ends in More, which opens the task.
const DESCRIPTION_LIMIT: usize = 240;

/// The start of a description, cut at a word and ended with an ellipsis when it's longer than `limit` characters.
/// The flag says whether anything was left out.
pub(super) fn shorten(text: &str, limit: usize) -> (String, bool) {
    let text = text.trim();
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    let cut: String = text.chars().take(limit).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(index) if index > 0 => cut[..index].to_string(),
        _ => cut,
    };
    (format!("{}\u{2026}", cut.trim_end()), true)
}

/// One line of the facts under a description: a quiet label, then its value.
fn fact(label: &str, value: String, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_row()
        .gap(theme.spacing.xs)
        .text_size(theme.typography.caption)
        .child(
            div()
                .text_color(theme.palette.text_secondary)
                .child(label.to_string()),
        )
        .child(div().font_weight(FontWeight::MEDIUM).child(value))
}

/// The details of an expanded card: the description, with More when it is long, the task's media, the facts
/// (priority, estimate, assignee, when it was last updated), and a way to have the AI explain the task. The card
/// above already shows the title and the tags.
fn card_details(
    task: &Task,
    cards: &CardContext,
    today: CalendarDate,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let id = task.id();
    let view = card_view(task, cards);
    let (description, truncated) = shorten(task.description(), DESCRIPTION_LIMIT);
    let dir = cx.global::<AppDatabase>().attachments_dir.clone();
    let updated = {
        let (year, month, day) =
            civil_from_days(task.updated_at().unix_millis().div_euclid(86_400_000));
        CalendarDate::new(year, month, day)
            .map(|date| format_due(date, today))
            .unwrap_or_else(|| "Unknown".to_string())
    };

    let mut details = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.sm)
        .mt(theme.spacing.xs)
        .pt(theme.spacing.sm)
        .border_t_1()
        .border_color(theme.palette.border)
        .cursor_default()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    if !description.is_empty() {
        let mut text = div().flex().flex_col().gap(theme.spacing.xs).child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .line_height(relative(1.5))
                .child(description),
        );
        if truncated {
            text = text.child(
                div()
                    .id(("card-more", id.as_uuid().as_u128() as u64))
                    .cursor_pointer()
                    .text_size(theme.typography.caption)
                    .font_weight(FontWeight::MEDIUM)
                    .hover(|style| style.text_color(theme.palette.text_primary))
                    .text_color(theme.palette.text_secondary)
                    .child("More")
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_panel(id, cx)),
                    ),
            );
        }
        details = details.child(text);
    }
    details = details.children(media::strip(id, &cards.media, &dir, false, theme, cx));

    let facts = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_x(theme.spacing.md)
        .gap_y(theme.spacing.xs)
        .child(fact(
            "Priority",
            view.priority
                .map(|priority| priority.label().trim_end_matches(" priority").to_string())
                .unwrap_or_else(|| "None".to_string()),
            theme,
        ))
        .child(fact(
            "Estimate",
            view.estimate_minutes
                .and_then(format_estimate)
                .unwrap_or_else(|| "None".to_string()),
            theme,
        ))
        .child(fact(
            "Assignee",
            view.assignee
                .as_ref()
                .map(|mark| mark.label.clone())
                .unwrap_or_else(|| "Unassigned".to_string()),
            theme,
        ))
        .child(fact("Updated", updated, theme));

    details
        .child(facts)
        .child(
            div()
                .id(("card-explain", id.as_uuid().as_u128() as u64))
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(theme.spacing.xs)
                .h(px(28.))
                .px(theme.spacing.sm)
                .self_start()
                .rounded(theme.radius.md)
                .border_1()
                .border_color(theme.palette.border)
                .cursor_pointer()
                .hover(|style| style.bg(theme.palette.surface_hover))
                .text_size(theme.typography.caption)
                .child(components::icon(
                    "sparkles",
                    14.,
                    theme.palette.text_secondary,
                ))
                .child("Explain issue")
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.explain_issue(id, window, cx)
                })),
        )
        .into_any_element()
}

impl WorkspaceView {
    /// Expands a card to show its details, or collapses it when it's already expanded. One card is open at a time.
    pub(super) fn toggle_card_details(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if self.expanded_card == Some(task) {
            self.expanded_card = None;
            self.expanded_media.clear();
        } else {
            self.expanded_card = Some(task);
            self.load_media(task, cx);
        }
        cx.notify();
    }
}

/// What the cards need from the view beyond their own task: the team, and the card editor that's
/// open, if any.
pub(super) struct CardContext {
    pub members: Vec<members::Entry>,
    pub expanded: Option<TaskId>,
    pub media: Vec<Attachment>,
    /// The search terms in use. Empty when nothing is being searched.
    pub terms: Vec<String>,
}

/// What the cards draw from the view, built fresh for each render so it can't go stale.
pub(super) fn card_context(view: &WorkspaceView) -> CardContext {
    CardContext {
        members: members::entries(&view.members),
        expanded: view.expanded_card,
        media: view.expanded_media.clone(),
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
        .border_color(theme.palette.border)
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
        hint: looji_domain::needs_estimate_hint(
            task.status(),
            task.due_on(),
            task.estimate().map(|estimate| estimate.minutes()),
        ),
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

/// The Columns control: a button that opens a menu with a checkbox for each column. Backlog and In review can be
/// hidden, and their tasks move to the nearest shown column. To Do, In progress and Done are always shown.
pub(super) fn columns_button(
    view: &WorkspaceView,
    project: &Project,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let trigger = crate::components::button_icon(
        "columns-menu-toggle",
        "layout-kanban",
        "Columns",
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_board_columns_menu(cx)));
    // Presses on the button and its menu stay with them, so the header doesn't close the menu first.
    let mut item = div()
        .relative()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);
    if view.board_columns_open {
        item = item.child(deferred(columns_menu(project, theme, cx)).with_priority(1));
    }
    item.into_any_element()
}

/// The menu under the Columns button: a checkbox for each column, and what can be hidden.
fn columns_menu(
    project: &Project,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let mut rows = div().flex().flex_col().gap(px(2.));
    for (index, status) in Status::ALL.into_iter().enumerate() {
        let shown = project.is_status_visible(status);
        let optional = matches!(status, Status::Backlog | Status::InReview);
        let row = div()
            .id(("column-option", index as u64))
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing.sm)
            .px(theme.spacing.sm)
            .py(theme.spacing.xs)
            .rounded(theme.radius.md)
            .child(crate::components::checkbox(shown, theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(status_label(status)),
            );
        let row = if optional {
            row.cursor_pointer()
                .text_color(theme.palette.text_primary)
                .hover(|style| style.bg(theme.palette.surface_hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_column(status, !shown, cx)
                }))
        } else {
            row.text_color(theme.palette.text_secondary)
        };
        rows = rows.child(row);
    }
    div()
        .id("columns-menu")
        .absolute()
        .text_size(theme.typography.body)
        .top(px(36.))
        .right_0()
        .w(px(248.))
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.sm)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .shadow(theme.elevation.overlay.clone())
        .occlude()
        .child(
            div()
                .px(theme.spacing.sm)
                .pt(theme.spacing.xs)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Board columns"),
        )
        .child(rows)
        .child(
            div()
                .px(theme.spacing.sm)
                .pb(theme.spacing.xs)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(
                    "To do, In progress and Done are always shown. Backlog and In review can be hidden; their tasks move to the nearest shown column.",
                ),
        )
}

/// The button that starts a new task in the first shown column. It becomes Cancel while the form is open.
pub(super) fn new_task_button(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let creating = view
        .task_dialog
        .as_ref()
        .is_some_and(|dialog| matches!(dialog.form.mode, Mode::Create));
    let (icon_name, label) = if creating {
        ("x", "Cancel")
    } else {
        ("plus", "New task")
    };
    crate::components::button_icon(
        "new-task",
        icon_name,
        label,
        crate::components::Variant::Secondary,
        theme,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
        this.open_task_dialog(Mode::Create, window, cx)
    }))
}

impl WorkspaceView {
    /// Opens or closes the Columns menu.
    pub(super) fn toggle_board_columns_menu(&mut self, cx: &mut Context<Self>) {
        self.board_columns_open = !self.board_columns_open;
        cx.notify();
    }

    /// Shows or hides a column from the Columns menu. The menu stays open, so several columns can be changed.
    fn toggle_column(&mut self, status: Status, visible: bool, cx: &mut Context<Self>) {
        self.request_column_visibility(status, visible, cx);
    }
}

#[cfg(test)]
mod description_tests {
    use super::shorten;

    #[test]
    fn a_short_description_is_shown_whole() {
        assert_eq!(
            shorten("  Fix the login.  ", 240),
            ("Fix the login.".to_string(), false)
        );
    }

    #[test]
    fn a_long_description_is_cut_at_a_word_and_says_so() {
        let (text, truncated) = shorten("alpha beta gamma delta", 12);
        assert!(truncated);
        assert_eq!(text, "alpha beta\u{2026}");
    }

    #[test]
    fn a_long_word_is_cut_in_place_and_emoji_are_not_split() {
        let (text, truncated) =
            shorten("\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}", 3);
        assert!(truncated);
        assert_eq!(text, "\u{1f600}\u{1f600}\u{1f600}\u{2026}");
    }
}
