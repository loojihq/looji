//! The board's filters: two selects, Priority and Assignee, that sit beside the search field. Each opens a
//! menu of its values, and "All priorities" or "All assignees" at its top means nothing is filtered by it.
//!
//! The filter belongs to the project. Every change is saved, and opening a project loads its own filter.
//! A selected value that no longer exists (a removed member) is dropped, so the bar never shows a selection
//! that matches nothing. Status and tag criteria saved by an earlier version are dropped on load, since
//! the bar no longer offers them, and a hidden criterion would still hide tasks.
//!
//! A menu is opened from the keyboard or the mouse. Up and Down move through its choices, Enter or Space
//! picks one, and Escape closes the menu and returns focus to its select.

use gpui::{prelude::*, *};
use looji_domain::{Priority, ProjectFilterRepository, TaskFilter, TeamMemberId};

use super::members;
use super::{WorkspaceView, store};
use crate::theme::Theme;

/// The focusable selects, in tab order: Priority, then Assignee.
pub(super) const CONTROL_COUNT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Facet {
    Priority,
    Assignee,
}

impl Facet {
    pub(super) const ALL: [Facet; 2] = [Facet::Priority, Facet::Assignee];

    /// The select's text while nothing is filtered by this facet, and the menu's first row.
    fn all_label(self) -> &'static str {
        match self {
            Facet::Priority => "All priorities",
            Facet::Assignee => "All assignees",
        }
    }

    pub(super) fn index(self) -> usize {
        Facet::ALL
            .iter()
            .position(|facet| *facet == self)
            .unwrap_or(0)
    }
}

/// One value a facet can select. `All` clears the facet, so nothing is filtered by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Choice {
    All(Facet),
    Priority(Option<Priority>),
    Assignee(Option<TeamMemberId>),
}

/// An open facet menu, and the choice the keyboard is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FilterMenu {
    pub facet: Facet,
    pub highlight: usize,
}

impl WorkspaceView {
    /// Loads the selected project's saved filter. Called when a project is opened, before its board
    /// loads, so the pruning that follows has the filter to work on.
    pub(super) fn load_project_filter(&mut self, cx: &mut Context<Self>) {
        self.filter_menu = None;
        let Some(project) = self.projects.active_id() else {
            self.filter = TaskFilter::default();
            return;
        };
        self.filter = match store(cx).load_filter(project) {
            Ok(filter) => filter,
            Err(err) => {
                eprintln!(
                    "could not load the filter for project {}: {err}",
                    project.as_uuid()
                );
                self.status = Some("Could not load this board's filters.".to_string());
                TaskFilter::default()
            }
        };
        // The bar offers only priority and assignee, so a status or tag saved earlier is dropped here.
        self.filter.statuses.clear();
        self.filter.tags.clear();
    }

    /// Drops selected values that the board no longer has, and saves the filter if anything was
    /// dropped. Runs whenever the board reloads, so a removed member can't linger.
    pub(super) fn prune_project_filter(&mut self, cx: &mut Context<Self>) {
        let members: Vec<TeamMemberId> = self.members.iter().map(|member| member.id()).collect();
        if self.filter.prune(&members, &[]) {
            self.save_project_filter(cx);
        }
    }

    fn save_project_filter(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        if let Err(err) = store(cx).save_filter(project, &self.filter) {
            eprintln!(
                "could not save the filter for project {}: {err}",
                project.as_uuid()
            );
            self.status = Some("Could not save this board's filters.".to_string());
        }
    }

    /// The values a facet offers: "All" first, then the facet's own values.
    pub(super) fn options(&self, facet: Facet) -> Vec<(Choice, String)> {
        let mut options = vec![(Choice::All(facet), facet.all_label().to_string())];
        match facet {
            Facet::Priority => options.extend(
                [
                    (None, "No priority"),
                    (Some(Priority::Low), "Low"),
                    (Some(Priority::Medium), "Medium"),
                    (Some(Priority::High), "High"),
                ]
                .into_iter()
                .map(|(priority, label)| (Choice::Priority(priority), label.to_string())),
            ),
            Facet::Assignee => {
                options.push((Choice::Assignee(None), "No assignee".to_string()));
                options.extend(
                    members::entries(&self.members)
                        .into_iter()
                        .map(|entry| (Choice::Assignee(Some(entry.id)), entry.label)),
                );
            }
        }
        options
    }

    /// Whether a choice is in effect. "All" is in effect when nothing is selected for its facet.
    fn is_selected(&self, choice: &Choice) -> bool {
        match choice {
            Choice::All(facet) => selected_count(&self.filter, *facet) == 0,
            Choice::Priority(priority) => self.filter.priorities.contains(priority),
            Choice::Assignee(assignee) => self.filter.assignees.contains(assignee),
        }
    }

    /// Selects or deselects a value, then saves. Cards that the change hides can't keep a keyboard move.
    fn apply_choice(&mut self, choice: Choice, cx: &mut Context<Self>) {
        match choice {
            Choice::All(Facet::Priority) => self.filter.priorities.clear(),
            Choice::All(Facet::Assignee) => self.filter.assignees.clear(),
            Choice::Priority(priority) => self.filter.toggle_priority(priority),
            Choice::Assignee(assignee) => self.filter.toggle_assignee(assignee),
        }
        self.save_project_filter(cx);
        self.cancel_keyboard_if_hidden(cx);
        self.list_query_changed(cx);
        cx.notify();
    }

    fn choose_option(&mut self, index: usize, choice: Choice, cx: &mut Context<Self>) {
        if let Some(menu) = self.filter_menu.as_mut() {
            menu.highlight = index;
        }
        self.apply_choice(choice, cx);
    }

    /// Opens a facet's menu, or closes it when it's already open.
    pub(super) fn toggle_filter_menu(&mut self, facet: Facet, cx: &mut Context<Self>) {
        self.filter_menu = match self.filter_menu {
            Some(menu) if menu.facet == facet => None,
            _ => Some(FilterMenu {
                facet,
                highlight: 0,
            }),
        };
        cx.notify();
    }

    /// A key while a facet menu is open. Returns whether the menu used it.
    pub(super) fn filter_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.filter_menu else {
            return false;
        };
        let options = self.options(menu.facet);
        let last = options.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "down" => self.move_menu_highlight((menu.highlight + 1).min(last)),
            "up" => self.move_menu_highlight(menu.highlight.saturating_sub(1)),
            "enter" | "space" => {
                if let Some((choice, _)) = options.into_iter().nth(menu.highlight) {
                    self.apply_choice(choice, cx);
                }
                return true;
            }
            "escape" => {
                self.filter_menu = None;
                self.filter_focus[menu.facet.index()].focus(window, cx);
            }
            "tab" => {
                self.filter_menu = None;
                return false;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn move_menu_highlight(&mut self, highlight: usize) {
        if let Some(menu) = self.filter_menu.as_mut() {
            menu.highlight = highlight;
        }
    }
}

/// How many values a facet has selected.
fn selected_count(filter: &TaskFilter, facet: Facet) -> usize {
    match facet {
        Facet::Priority => filter.priorities.len(),
        Facet::Assignee => filter.assignees.len(),
    }
}

/// The two filter selects, in a row. Each select's text names what it filters by, or says "All" while
/// nothing is selected for it. The row keeps the press, so a click on a select doesn't close its open
/// menu through the board's own press handling.
pub(super) fn facets(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let mut row = div()
        .id("filter-facets")
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(theme.spacing.sm)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    for facet in Facet::ALL {
        // The select names what is chosen, or says "All" while nothing is. Several choices are listed together.
        let chosen: Vec<String> = view
            .options(facet)
            .into_iter()
            .filter(|(choice, _)| !matches!(choice, Choice::All(_)) && view.is_selected(choice))
            .map(|(_, label)| label)
            .collect();
        let label = if chosen.is_empty() {
            facet.all_label().to_string()
        } else {
            chosen.join(", ")
        };
        let open = view.filter_menu.is_some_and(|menu| menu.facet == facet);
        let trigger =
            crate::components::select(("filter-facet", facet.index() as u64), &label, open, theme)
                .track_focus(&view.filter_focus[facet.index()])
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_filter_menu(facet, cx)
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.toggle_filter_menu(facet, cx);
                    }
                }));
        // The menu floats over the page below its own select, so opening it moves nothing.
        let mut item = div().relative().child(trigger);
        if open {
            let rows = menu_rows(view, facet, theme, cx);
            item = item.child(
                deferred(crate::components::dropdown_menu(
                    ("filter-menu", facet.index() as u64),
                    rows,
                    theme,
                ))
                .with_priority(1),
            );
        }
        row = row.child(item);
    }
    row.into_any_element()
}

/// The empty result notice: shown when the filters hide every card. Nothing here clears them. The menus
/// have "All" rows for that.
pub(super) fn empty_notice(
    view: &WorkspaceView,
    shown: usize,
    theme: &Theme,
) -> Option<AnyElement> {
    if shown > 0 || view.board.is_empty() || view.filter.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .px(theme.spacing.lg)
            .py(theme.spacing.sm)
            .text_color(theme.palette.text_secondary)
            .child("No tasks match these filters.")
            .into_any_element(),
    )
}

/// The rows of a facet's menu: "All" first, then one per value. A chosen value is ringed, and the one the
/// keys are on is highlighted. Clicking a row chooses it.
fn menu_rows(
    view: &WorkspaceView,
    facet: Facet,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Vec<AnyElement> {
    let highlight = view
        .filter_menu
        .filter(|menu| menu.facet == facet)
        .map(|menu| menu.highlight);
    view.options(facet)
        .into_iter()
        .enumerate()
        .map(|(index, (choice, label))| {
            let selected = view.is_selected(&choice);
            crate::components::dropdown_row(
                ("filter-option", index as u64),
                &label,
                selected,
                highlight == Some(index),
                theme,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.choose_option(index, choice.clone(), cx)
            }))
            .into_any_element()
        })
        .collect()
}
