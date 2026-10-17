//! The command palette, opened with Ctrl+K. It finds tasks and projects by name and runs the
//! actions the app already has. While it is open it takes the keyboard.

use gpui::{prelude::*, *};
use looji_domain::{ProjectId, TaskId};

use super::list_view::ViewMode;
use super::task_form::Mode;
use super::{WorkspaceView, typed_text};
use crate::{components, theme::Theme};

/// How many tasks the palette lists at most, so a large board doesn't bury the actions.
const MAX_TASKS: usize = 8;

/// What the palette is showing: the query typed so far, and which row the keys are on.
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    query: String,
    selected: usize,
}

#[derive(Clone)]
enum Action {
    Task(TaskId),
    Project(ProjectId),
    View(ViewMode),
    NewTask,
    NewProject,
    Search,
    Export,
    Plan,
}

struct Item {
    group: &'static str,
    label: String,
    detail: Option<String>,
    action: Action,
}

/// The rows that match the query, in the order they are shown. Project-scoped actions only appear
/// when a project is open.
fn items(view: &WorkspaceView, query: &str) -> Vec<Item> {
    let needle = query.trim().to_lowercase();
    let matches = |text: &str| needle.is_empty() || text.to_lowercase().contains(&needle);
    let project = view.projects.active();

    let mut items = Vec::new();
    for task in view
        .board
        .iter()
        .filter(|task| matches(task.title()))
        .take(MAX_TASKS)
    {
        items.push(Item {
            group: "Tasks",
            label: task.title().to_string(),
            detail: project.map(|project| project.name().to_string()),
            action: Action::Task(task.id()),
        });
    }
    for project in view
        .projects
        .items()
        .iter()
        .filter(|project| matches(project.name()))
    {
        items.push(Item {
            group: "Projects",
            label: project.name().to_string(),
            detail: None,
            action: Action::Project(project.id()),
        });
    }

    let mut actions = vec![("New project", Action::NewProject)];
    if project.is_some() {
        actions.extend([
            ("New task", Action::NewTask),
            ("Search the board", Action::Search),
            ("Plan a project", Action::Plan),
            ("Export PDF", Action::Export),
            ("Go to board", Action::View(ViewMode::Board)),
            ("Go to list", Action::View(ViewMode::List)),
            ("Go to roadmap", Action::View(ViewMode::Roadmap)),
            ("Go to workload", Action::View(ViewMode::Workload)),
        ]);
    }
    for (label, action) in actions {
        if matches(label) {
            items.push(Item {
                group: "Actions",
                label: label.to_string(),
                detail: None,
                action,
            });
        }
    }
    items
}

impl WorkspaceView {
    /// Opens the palette with an empty query.
    pub(super) fn open_palette(&mut self, cx: &mut Context<Self>) {
        self.palette = Some(PaletteState::default());
        cx.notify();
    }

    fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        cx.notify();
    }

    /// Runs one row. The palette closes first, so the action opens into a clean window.
    fn run_palette_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(cx);
        match action {
            Action::Task(id) => self.open_panel(id, cx),
            Action::Project(id) => self.select_project(id, cx),
            Action::View(mode) => self.set_view_mode(mode, cx),
            Action::NewTask => self.open_task_dialog(Mode::Create, window, cx),
            Action::NewProject => self.open_project_dialog(window, cx),
            Action::Search => self.focus_search(window, cx),
            Action::Export => self.export_report(cx),
            Action::Plan => self.open_plan_wizard(window, cx),
        }
    }

    /// Keys while the palette is open: typing filters, the arrows move, Enter runs, Escape closes.
    pub(super) fn palette_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.palette.clone() else {
            return;
        };
        let list = items(self, &state.query);
        let count = list.len();
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => return self.close_palette(cx),
            "enter" => {
                if let Some(item) = list.get(state.selected) {
                    let action = item.action.clone();
                    self.run_palette_action(action, window, cx);
                }
                return;
            }
            "down" | "up" => {
                if count == 0 {
                    return;
                }
                let step = if key == "down" { 1 } else { count - 1 };
                self.palette = Some(PaletteState {
                    selected: (state.selected + step) % count,
                    ..state
                });
            }
            "backspace" => {
                let mut query = state.query;
                query.pop();
                self.palette = Some(PaletteState { query, selected: 0 });
            }
            _ => {
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                if let Some(text) = typed_text(&event.keystroke) {
                    let mut query = state.query;
                    query.push_str(text);
                    self.palette = Some(PaletteState { query, selected: 0 });
                }
            }
        }
        cx.notify();
    }
}

/// The palette drawn over the window, or nothing when it is closed.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let state = view.palette.as_ref()?;
    let list = items(view, &state.query);

    let mut rows = div().flex().flex_col().py(theme.spacing.xs);
    let mut group = "";
    for (index, item) in list.iter().enumerate() {
        if item.group != group {
            group = item.group;
            rows = rows.child(
                div()
                    .px(theme.spacing.lg)
                    .pt(theme.spacing.sm)
                    .pb(theme.spacing.xs)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child(group),
            );
        }
        let action = item.action.clone();
        let selected = index == state.selected;
        rows = rows.child(
            div()
                .id(("palette-row", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .mx(theme.spacing.xs)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .cursor_pointer()
                .when(selected, |row| row.bg(theme.palette.surface_selected))
                .when(!selected, |row| {
                    row.hover(|style| style.bg(theme.palette.surface_hover))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(item.label.clone()),
                )
                .children(item.detail.clone().map(|detail| {
                    div()
                        .text_size(theme.typography.caption)
                        .text_color(theme.palette.text_secondary)
                        .child(detail)
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.run_palette_action(action.clone(), window, cx);
                })),
        );
    }
    if list.is_empty() {
        rows = rows.child(
            div()
                .px(theme.spacing.lg)
                .py(theme.spacing.sm)
                .text_color(theme.palette.text_secondary)
                .child("Nothing matches that."),
        );
    }

    let query_text = if state.query.is_empty() {
        div()
            .text_color(theme.palette.text_secondary)
            .child("Search tasks, projects, and actions")
    } else {
        div().child(format!("{}|", state.query))
    };
    let input = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.lg)
        .py(theme.spacing.md)
        .border_b_1()
        .border_color(theme.palette.border)
        .child(components::icon(
            "search",
            16.,
            theme.palette.text_secondary,
        ))
        .child(div().flex_1().min_w_0().child(query_text))
        .child(components::kbd("Esc", theme));

    let footer = div()
        .flex()
        .flex_row()
        .gap(theme.spacing.lg)
        .px(theme.spacing.lg)
        .py(theme.spacing.sm)
        .border_t_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child("↑ ↓  Navigate")
        .child("Enter  Open");

    let panel = div()
        .id("palette-panel")
        .flex()
        .flex_col()
        .w(px(480.))
        .rounded(theme.radius.lg)
        .overflow_hidden()
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .shadow(theme.elevation.overlay.clone())
        // A click inside the box stays in the box; only a click on the backdrop closes it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(input)
        .child(rows)
        .child(footer);

    Some(
        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(72.))
            .bg(theme.palette.scrim)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_palette(cx)),
            )
            .child(panel)
            .into_any_element(),
    )
}
