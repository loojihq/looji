//! The board's filter bar: four facets (status, priority, assignee, and tag), each a menu of checkboxes,
//! a count of the facets in use, and one control that clears them all.
//!
//! The filter belongs to the project. Every change is saved, and opening a project loads its own filter.
//! A selected value that no longer exists (a removed member, a tag no task uses) is dropped, so the bar
//! never shows a selection that matches nothing.
//!
//! A facet's menu is opened from the keyboard or the mouse. Up and Down move through its choices, Enter
//! or Space picks one, and Escape closes the menu and returns focus to its button.

use gpui::{prelude::*, *};
use looji_domain::{Priority, ProjectFilterRepository, Status, TagMatch, TaskFilter, TeamMemberId};

use super::board::status_label;
use super::members;
use super::{WorkspaceView, store};
use crate::theme::Theme;

/// The focusable controls in the bar, in tab order: the four facets, then the clear control.
pub(super) const CONTROL_COUNT: usize = 5;
pub(super) const CLEAR_CONTROL: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Facet {
    Status,
    Priority,
    Assignee,
    Tag,
}

impl Facet {
    pub(super) const ALL: [Facet; 4] =
        [Facet::Status, Facet::Priority, Facet::Assignee, Facet::Tag];

    fn label(self) -> &'static str {
        match self {
            Facet::Status => "Status",
            Facet::Priority => "Priority",
            Facet::Assignee => "Assignee",
            Facet::Tag => "Tag",
        }
    }

    pub(super) fn index(self) -> usize {
        Facet::ALL
            .iter()
            .position(|facet| *facet == self)
            .unwrap_or(0)
    }
}

/// One value a facet can select.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Choice {
    Status(Status),
    Priority(Option<Priority>),
    Assignee(Option<TeamMemberId>),
    Tag(String),
    /// Whether a task must match every selected tag, rather than any of them.
    TagMode,
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
    }

    /// Drops selected values that the board no longer has, and saves the filter if anything was
    /// dropped. Runs whenever the board reloads, so a removed member or an unused tag can't linger.
    pub(super) fn prune_project_filter(&mut self, cx: &mut Context<Self>) {
        let members: Vec<TeamMemberId> = self.members.iter().map(|member| member.id()).collect();
        let mut tags: Vec<String> = self
            .board
            .iter()
            .flat_map(|task| task.tags().iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        if self.filter.prune(&members, &tags) {
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

    /// The values a facet offers. Statuses are the ones the project shows, tags are the ones its tasks
    /// use, and assignees are the team, with "No assignee" first.
    pub(super) fn options(&self, facet: Facet) -> Vec<(Choice, String)> {
        match facet {
            Facet::Status => self
                .projects
                .active()
                .map(|project| project.visible_statuses())
                .unwrap_or_default()
                .into_iter()
                .map(|status| (Choice::Status(status), status_label(status).to_string()))
                .collect(),
            Facet::Priority => [
                (None, "No priority"),
                (Some(Priority::Low), "Low"),
                (Some(Priority::Medium), "Medium"),
                (Some(Priority::High), "High"),
            ]
            .into_iter()
            .map(|(priority, label)| (Choice::Priority(priority), label.to_string()))
            .collect(),
            Facet::Assignee => {
                let mut options = vec![(Choice::Assignee(None), "No assignee".to_string())];
                options.extend(
                    members::entries(&self.members)
                        .into_iter()
                        .map(|entry| (Choice::Assignee(Some(entry.id)), entry.label)),
                );
                options
            }
            Facet::Tag => {
                let mut tags: Vec<String> = self
                    .board
                    .iter()
                    .flat_map(|task| task.tags().iter().cloned())
                    .collect();
                tags.sort();
                tags.dedup();
                let mut options: Vec<(Choice, String)> = tags
                    .into_iter()
                    .map(|tag| (Choice::Tag(tag.clone()), tag))
                    .collect();
                options.push((Choice::TagMode, "Require every selected tag".to_string()));
                options
            }
        }
    }

    fn is_selected(&self, choice: &Choice) -> bool {
        match choice {
            Choice::Status(status) => self.filter.statuses.contains(status),
            Choice::Priority(priority) => self.filter.priorities.contains(priority),
            Choice::Assignee(assignee) => self.filter.assignees.contains(assignee),
            Choice::Tag(tag) => self.filter.tags.contains(tag),
            Choice::TagMode => self.filter.tag_match == TagMatch::All,
        }
    }

    /// Selects or deselects a value, then saves. Cards that the change hides can't keep a keyboard move.
    fn apply_choice(&mut self, choice: Choice, cx: &mut Context<Self>) {
        match choice {
            Choice::Status(status) => self.filter.toggle_status(status),
            Choice::Priority(priority) => self.filter.toggle_priority(priority),
            Choice::Assignee(assignee) => self.filter.toggle_assignee(assignee),
            Choice::Tag(tag) => self.filter.toggle_tag(&tag),
            Choice::TagMode => {
                self.filter.tag_match = match self.filter.tag_match {
                    TagMatch::Any => TagMatch::All,
                    TagMatch::All => TagMatch::Any,
                };
            }
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

    /// Clears every facet and the tag mode. The search is left alone.
    pub(super) fn clear_filters(&mut self, cx: &mut Context<Self>) {
        if self.filter.is_empty() {
            return;
        }
        self.filter.clear();
        self.filter_menu = None;
        self.save_project_filter(cx);
        self.list_query_changed(cx);
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
                self.filter_focus[menu.facet.index()].focus(window);
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
        Facet::Status => filter.statuses.len(),
        Facet::Priority => filter.priorities.len(),
        Facet::Assignee => filter.assignees.len(),
        Facet::Tag => filter.tags.len(),
    }
}

/// The filter bar above the board. Its controls keep the press, so a click on them doesn't close the
/// open menu through the board's own press handling.
pub(super) fn bar(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let active = view.filter.active_facets();
    let badge = if active == 0 {
        "Filters".to_string()
    } else {
        // The number counts facets, not values, so "Filters (2)" is two facets with a value each.
        format!("Filters ({active})")
    };

    let mut facets = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(theme.spacing.xs);
    for facet in Facet::ALL {
        let selected = selected_count(&view.filter, facet);
        let label = match selected {
            0 => facet.label().to_string(),
            count => format!("{} · {count}", facet.label()),
        };
        let open = view.filter_menu.is_some_and(|menu| menu.facet == facet);
        facets = facets.child(
            crate::components::pill(("filter-facet", facet.index() as u64), &label, open, theme)
                .track_focus(&view.filter_focus[facet.index()])
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_filter_menu(facet, cx)
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.toggle_filter_menu(facet, cx);
                    }
                })),
        );
    }

    let clearable = active > 0;
    let clear = crate::components::pill(("filter-clear", 0_u64), "Clear filters", false, theme)
        .track_focus(&view.filter_focus[CLEAR_CONTROL])
        .when(!clearable, |button| {
            button.text_color(theme.palette.text_disabled)
        })
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_filters(cx)))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                this.clear_filters(cx);
            }
        }));

    let mut bar = div()
        .id("filter-bar")
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .px(theme.spacing.lg)
        .py(theme.spacing.xs)
        .border_b_1()
        .border_color(theme.palette.border)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(theme.spacing.sm)
                .child(
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(badge),
                )
                .child(facets)
                .child(clear),
        );

    if let Some(menu) = view.filter_menu {
        bar = bar.child(menu_panel(view, menu, theme, cx));
    }
    bar.into_any_element()
}

/// The open facet menu: one row per choice, with a tick on the selected ones and a highlight on the one
/// the keyboard is on.
fn menu_panel(
    view: &WorkspaceView,
    menu: FilterMenu,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let mut list = div()
        .id("filter-menu")
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .p(theme.spacing.xs)
        .max_h(px(240.))
        .overflow_y_scroll()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .bg(theme.palette.surface_raised);
    for (index, (choice, label)) in view.options(menu.facet).into_iter().enumerate() {
        let selected = view.is_selected(&choice);
        let highlighted = menu.highlight == index;
        list = list.child(
            div()
                .id(("filter-option", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.sm)
                .cursor_pointer()
                .when(highlighted, |row| row.bg(theme.palette.surface_selected))
                .hover(|style| style.bg(theme.palette.surface_hover))
                .child(crate::components::check_mark(selected, theme))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_option(index, choice.clone(), cx)
                })),
        );
    }
    list.into_any_element()
}

/// The note shown when the filters hide every task of a project that has tasks. The columns stay, and
/// each says "No matches", so the board still shows where the cards would be. A project with no tasks
/// at all shows nothing here, and keeps its own empty state.
pub(super) fn empty_notice(
    view: &WorkspaceView,
    shown: usize,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    if shown > 0 || view.board.is_empty() || view.filter.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(theme.spacing.lg)
            .py(theme.spacing.sm)
            .child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child("No tasks match these filters."),
            )
            .child(
                div()
                    .id(("filter-clear", 1_u64))
                    .px(theme.spacing.sm)
                    .py(theme.spacing.xs)
                    .rounded(theme.radius.md)
                    .cursor_pointer()
                    .border_1()
                    .border_color(theme.palette.border_control)
                    .hover(|style| style.bg(theme.palette.surface_hover))
                    .child("Clear filters")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_filters(cx))),
            )
            .into_any_element(),
    )
}
