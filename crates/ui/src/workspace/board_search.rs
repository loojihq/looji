//! The board's search field: what's typed, the query it filters by, and the keys and focus that go with
//! it. The matching itself is in `search.rs`. The query is held in memory and never written to the
//! database.
//!
//! Typing changes the draft at once. The board filters by the query after a short pause, so a fast
//! typist re-filters once rather than on every key. Clearing the field, or pressing Escape, takes
//! effect at once.

use std::borrow::Cow;
use std::time::Duration;

use gpui::{prelude::*, *};
use looji_domain::Task;

use super::WorkspaceView;
use super::drag::DropResult;
use super::keyboard::Hold;
use super::search;
use crate::{text_edit::TextEdit, theme::Theme};

/// How long the board waits after the last keystroke before it filters.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(180);

impl WorkspaceView {
    /// Puts the keyboard in the search field. Keys go to the field until it loses focus.
    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_focused = true;
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Takes the keyboard out of the field. The text and the filter stay as they are.
    pub(super) fn blur_search(&mut self, cx: &mut Context<Self>) {
        if self.search_focused {
            self.search_focused = false;
            cx.notify();
        }
    }

    /// Empties the field and the filter at once. A pause still waiting is made stale, so it can't
    /// bring back the old query. Focus goes to the board.
    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_draft.clear();
        self.search_debounce.touch();
        self.set_search_query(String::new(), cx);
        self.search_focused = false;
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// A new project has its own board, so its search starts empty.
    pub(super) fn reset_search(&mut self) {
        self.search_draft.clear();
        self.search_query.clear();
        self.search_debounce.touch();
    }

    /// Changes the text in the field. The filter follows after a short pause, and only the last of
    /// several quick changes applies.
    fn edit_search_draft(&mut self, change: impl FnOnce(&mut TextEdit), cx: &mut Context<Self>) {
        change(&mut self.search_draft);
        let number = self.search_debounce.touch();
        let text = self.search_draft.as_str().to_string();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |view, cx| {
                if view.search_debounce.is_latest(number) {
                    view.set_search_query(text, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Applies a query. A card that's now hidden can't keep a keyboard move in progress.
    fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.search_query {
            return;
        }
        self.search_query = query;
        self.cancel_keyboard_if_hidden(cx);
        self.list_query_changed(cx);
        cx.notify();
    }

    pub(super) fn cancel_keyboard_if_hidden(&mut self, cx: &mut Context<Self>) {
        let terms = search::terms(&self.search_query);
        let Hold::Holding { task, .. } = self.drag.keyboard else {
            return;
        };
        let hidden = self
            .board
            .iter()
            .find(|other| other.id() == task)
            .is_some_and(|other| !self.shows(other, &terms));
        if hidden {
            self.drag.keyboard = Hold::Idle;
            self.drag.session.clear_hold();
            self.drag.keyboard_activation = None;
            self.drag.keyboard_notice = Some(
                "That card is hidden by the search or filters, so the move was cancelled."
                    .to_string(),
            );
            cx.notify();
        }
    }

    /// Whether the board shows a task: it passes the filters and the search. Everything that decides
    /// visibility goes through here, so the board, drops, and keyboard moves agree.
    pub(super) fn shows(&self, task: &Task, terms: &[String]) -> bool {
        self.filter.matches(task) && search::matches(task, terms)
    }

    /// A key while the field has the keyboard. Returns whether the field used it. Tab leaves the field
    /// and isn't used, so focus moves on the usual way.
    pub(super) fn search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match event.keystroke.key.as_str() {
            "escape" => self.clear_search(window, cx),
            "enter" => self.blur_search(cx),
            "tab" => {
                self.blur_search(cx);
                return false;
            }
            _ => {
                let keystroke = event.keystroke.clone();
                let mut handled = false;
                self.edit_search_draft(|draft| handled = draft.handle_key(&keystroke), cx);
                return handled;
            }
        }
        true
    }

    /// The index a drop lands at among all of its column's cards, rather than among the visible
    /// ones. With nothing hiding cards, the index is already the full one.
    pub(super) fn full_drop_index(&self, drop: &DropResult) -> usize {
        let terms = search::terms(&self.search_query);
        if terms.is_empty() && self.filter.is_empty() {
            return drop.index;
        }
        let others: Vec<&Task> = self
            .board
            .iter()
            .filter(|task| task.status() == drop.status && task.id() != drop.task)
            .collect();
        let full: Vec<_> = others.iter().map(|task| task.id()).collect();
        let visible: Vec<_> = others
            .iter()
            .filter(|task| self.shows(task, &terms))
            .map(|task| task.id())
            .collect();
        search::full_index(&full, &visible, drop.index)
    }
}

/// The tasks the board shows, in their order. It's computed once per render of the board, not once per
/// card, and nothing is copied when no filter or search is running.
pub(super) fn shown_tasks(view: &WorkspaceView) -> Cow<'_, [Task]> {
    let terms = search::terms(&view.search_query);
    if terms.is_empty() && view.filter.is_empty() {
        return Cow::Borrowed(&view.board);
    }
    Cow::Owned(
        view.board
            .iter()
            .filter(|task| view.shows(task, &terms))
            .cloned()
            .collect(),
    )
}

/// The search field and what it says about the board: how many cards are showing. Shown above the
/// board.
pub(super) fn bar(
    view: &WorkspaceView,
    shown: usize,
    total: usize,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let searching = !search::terms(&view.search_query).is_empty();
    let count = match (searching, shown) {
        (false, _) => String::new(),
        (true, 0) => "No matches".to_string(),
        (true, _) => format!("{shown} of {total}"),
    };
    let field = div()
        .id("board-search")
        .flex_1()
        .max_w(px(360.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                // The field keeps the press, so the board doesn't take it as a click elsewhere.
                cx.stop_propagation();
                this.focus_search(window, cx)
            }),
        )
        .child(crate::components::search_field(
            &view.search_draft,
            "Search cards (Ctrl+F)",
            view.search_focused,
            theme,
        ));
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.lg)
        .py(theme.spacing.xs)
        .border_b_1()
        .border_color(theme.palette.border)
        .child(field)
        .child(super::filter_bar::facets(view, theme, cx))
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(count),
        );
    if !view.search_draft.is_empty() {
        row = row.child(
            div()
                .id("board-search-clear")
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .cursor_pointer()
                .border_1()
                .border_color(theme.palette.border_control)
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child("Clear")
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.clear_search(window, cx)),
                ),
        );
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use super::SEARCH_DEBOUNCE;

    #[test]
    fn the_pause_is_short_but_not_zero() {
        assert!(SEARCH_DEBOUNCE.as_millis() >= 150);
        assert!(SEARCH_DEBOUNCE.as_millis() <= 250);
    }
}
