//! The main window: a title bar over a sidebar of workspaces and projects, and the main pane.

mod autoscroll;
mod avatar;
mod board;
mod board_search;
mod card;
mod columns;
mod delete;
mod drag;
mod dragging;
mod fields;
mod filter_bar;
mod keyboard;
mod landing;
mod layout;
mod list_view;
mod members;
mod moves;
mod panel;
mod preferences_panel;
mod project_state;
mod projects;
mod search;
mod sidebar;
mod state;
mod task_dialog;
mod task_edit;
mod task_editors;
mod task_form;
mod team_dialog;

use delete::PendingDelete;
use project_state::ProjectList;
use projects::ProjectDialog;

use gpui::{prelude::*, *};
use looji_domain::{
    DomainError, RepoError, SettingsRepository, Task, TaskFilter, TaskId, TeamMember, Workspace,
    WorkspaceId, WorkspaceRepository, next_untitled_name, validate_name,
};
use looji_persistence::icons::{IconError, import_icon};

pub use state::WorkspaceList;

use crate::{
    APP_NAME,
    database::AppDatabase,
    theme::{ActiveTheme, Theme},
    title_bar::TitleBar,
};

const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1280.), px(800.));
/// Below this the sidebar and main pane can't both render usefully.
const MIN_WINDOW_SIZE: Size<Pixels> = size(px(640.), px(400.));
const NAME_TAKEN_MESSAGE: &str = "That name is already taken.";

/// A workspace whose name is being edited inline in the sidebar.
#[derive(Debug, Clone)]
struct Rename {
    id: WorkspaceId,
    draft: String,
    error: Option<String>,
}

pub struct WorkspaceView {
    focus_handle: FocusHandle,
    list: WorkspaceList,
    rename: Option<Rename>,
    /// Failures that aren't tied to one row, such as a switch that couldn't be saved.
    status: Option<String>,
    /// The delete confirmation, while it's open.
    pending_delete: Option<PendingDelete>,
    /// The projects of the active workspace.
    projects: ProjectList,
    /// The tasks of the selected project. Empty when no project is selected.
    board: Vec<Task>,
    /// The team of the selected project, in the order they were added. Empty when no project is
    /// selected.
    members: Vec<TeamMember>,
    /// The create-project dialog, while it's open.
    project_dialog: Option<ProjectDialog>,
    /// A column hide waiting for confirmation.
    pending_column: Option<columns::PendingColumn>,
    /// The card drag in progress, and the last drag's outcome.
    drag: dragging::DragSession,
    /// The create or edit task dialog, while it's open.
    task_dialog: Option<task_dialog::TaskDialog>,
    /// The team dialog, while it's open.
    team_dialog: Option<team_dialog::TeamDialog>,
    /// The task detail panel beside the board, while a task is open in it.
    panel: Option<panel::PanelState>,
    /// The tag being typed for a task, while the tag input has the keyboard.
    tag_edit: Option<task_edit::TagEdit>,
    /// The card whose priority and tag editor is open.
    card_editing: Option<TaskId>,
    /// The board search: what is typed, the query the board filters by, and whether the field has
    /// the keyboard.
    search_draft: String,
    search_query: String,
    search_focused: bool,
    search_debounce: search::Debounce,
    /// Whether the board area shows the board or the list, and the list's paging state.
    view_mode: list_view::ViewMode,
    task_list: list_view::ListState,
    /// The board filter of the selected project, and the facet menu that is open, if any.
    filter: TaskFilter,
    filter_menu: Option<filter_bar::FilterMenu>,
    /// One focus handle for each control in the filter bar, in tab order.
    filter_focus: Vec<FocusHandle>,
}

impl WorkspaceView {
    pub fn open_window(cx: &mut App, list: WorkspaceList) -> Result<WindowHandle<Self>> {
        cx.open_window(window_options(cx), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = Self {
                    focus_handle: cx.focus_handle(),
                    list,
                    rename: None,
                    status: None,
                    pending_delete: None,
                    projects: ProjectList::empty(),
                    board: Vec::new(),
                    members: Vec::new(),
                    project_dialog: None,
                    pending_column: None,
                    drag: dragging::DragSession::default(),
                    task_dialog: None,
                    team_dialog: None,
                    panel: None,
                    tag_edit: None,
                    card_editing: None,
                    search_draft: String::new(),
                    search_query: String::new(),
                    search_focused: false,
                    search_debounce: search::Debounce::default(),
                    view_mode: list_view::ViewMode::default(),
                    task_list: list_view::ListState::default(),
                    filter: TaskFilter::default(),
                    filter_menu: None,
                    filter_focus: (0..filter_bar::CONTROL_COUNT)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                };
                view.reload_projects(cx);
                view
            });
            // Focus starts on the workspace, so keys reach it, including Tab for the board.
            let focus = view.read(cx).focus_handle.clone();
            focus.focus(window);
            view
        })
    }

    /// Clicking a workspace ends any rename in progress, so it acts as the "blur" for the editor.
    fn switch_to(&mut self, id: WorkspaceId, cx: &mut Context<Self>) {
        self.cancel_rename(cx);
        if id == self.list.active_workspace_id() {
            return;
        }
        // Write first; only move the selection once the write has succeeded.
        match SettingsRepository::set_active_workspace(store(cx), id) {
            Ok(()) => {
                self.list.set_active(id);
                // The previous workspace's project must not stay selected.
                self.reload_projects(cx);
            }
            Err(err) => self.status = Some(format!("Could not switch workspace: {err}")),
        }
        cx.notify();
    }

    /// Creates a workspace with a generated name and starts renaming it.
    fn create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = next_untitled_name(self.list.items());
        let workspace = Workspace::new(&name).expect("generated names are valid");
        match WorkspaceRepository::save(store(cx), &workspace) {
            Ok(()) => {
                let id = workspace.id();
                self.list.push(workspace);
                self.status = None;
                self.begin_rename(id, window, cx);
            }
            Err(err) => {
                self.status = Some(format!("Could not create a workspace: {err}"));
                cx.notify();
            }
        }
    }

    fn begin_rename(&mut self, id: WorkspaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.list.find(id) else {
            return;
        };
        self.rename = Some(Rename {
            id,
            draft: workspace.name().to_string(),
            error: None,
        });
        self.focus_handle.focus(window);
        cx.notify();
    }

    /// Discards the draft. The stored name is untouched.
    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    /// Validates the draft and saves it. On failure the editor stays open with a message.
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.clone() else {
            return;
        };
        let Some(current) = self.list.find(rename.id).cloned() else {
            self.cancel_rename(cx);
            return;
        };

        let cleaned = match validate_name(&rename.draft, self.list.items(), Some(rename.id)) {
            Ok(name) => name,
            Err(err) => return self.reject_rename(rename_message(&err), cx),
        };
        if cleaned == current.name() {
            return self.cancel_rename(cx);
        }

        let mut updated = current;
        if let Err(err) = updated.rename(&cleaned) {
            return self.reject_rename(rename_message(&err), cx);
        }
        match WorkspaceRepository::save(store(cx), &updated) {
            Ok(()) => {
                self.list.replace(updated);
                self.rename = None;
                cx.notify();
            }
            Err(RepoError::NameTaken) => self.reject_rename(NAME_TAKEN_MESSAGE.into(), cx),
            Err(err) => self.reject_rename(format!("Could not save the name: {err}"), cx),
        }
    }

    fn reject_rename(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(rename) = self.rename.as_mut() {
            rename.error = Some(message);
        }
        cx.notify();
    }

    /// Asks for an image, imports it on a background thread, then records it. The icon only
    /// changes if the import and the database write both succeed.
    fn choose_icon(&mut self, id: WorkspaceId, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose icon".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(source) = paths.into_iter().next() else {
                return;
            };
            let Ok(icons_dir) = cx.update(|cx| cx.global::<AppDatabase>().icons_dir.clone()) else {
                return;
            };
            let imported = cx
                .background_executor()
                .spawn(async move { import_icon(&source, &icons_dir, id) })
                .await;
            let _ = this.update(cx, |view, cx| view.apply_icon(id, imported, cx));
        })
        .detach();
    }

    fn apply_icon(
        &mut self,
        id: WorkspaceId,
        imported: Result<String, IconError>,
        cx: &mut Context<Self>,
    ) {
        let file_name = match imported {
            Ok(file_name) => file_name,
            Err(err) => {
                self.status = Some(format!("Could not use that image: {err}"));
                return cx.notify();
            }
        };
        let icons_dir = cx.global::<AppDatabase>().icons_dir.clone();
        match store(cx).replace_workspace_icon(&icons_dir, id, Some(&file_name)) {
            Ok(()) => {
                // Validated by `import_icon`, so the file name is always accepted here.
                if let Some(mut updated) = self.list.find(id).cloned()
                    && updated.set_icon_file(Some(file_name)).is_ok()
                {
                    self.list.replace(updated);
                }
                self.status = None;
            }
            Err(err) => self.status = Some(format!("Could not change the icon: {err}")),
        }
        cx.notify();
    }

    /// Switches between light and dark mode. Every window re-renders with the new palette.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = cx.theme().mode.toggled();
        // Save first, so the screen never shows a theme that wasn't stored.
        match crate::settings::update(cx, looji_domain::Preference::Theme(next.into())) {
            Ok(()) => crate::theme::set_mode(next, cx),
            Err(err) => self.status = Some(format!("Could not save the theme: {err}")),
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The search field takes its keys first, so shortcuts and card moves don't fire while typing.
        if self.search_focused && self.search_key(event, window, cx) {
            return;
        }
        if self.filter_menu.is_some() && self.filter_key(event, window, cx) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if event.keystroke.key == "f" && (modifiers.control || modifiers.platform) {
            return self.focus_search(window, cx);
        }
        if self.key_pressed(event, cx) {
            return;
        }
        if self.task_dialog.is_some() {
            return self.task_dialog_key(event, cx);
        }
        if self.team_dialog.is_some() {
            return self.team_dialog_key(event, cx);
        }
        if self.tag_edit.is_some() && self.tag_key(event, cx) {
            return;
        }
        if self.panel.is_some() && self.panel_key(event, window, cx) {
            return;
        }
        // Tab moves between cards and other controls, unless a dialog or a rename owns the keyboard.
        let modal = self.pending_column.is_some()
            || self.project_dialog.is_some()
            || self.pending_delete.is_some()
            || self.rename.is_some();
        if self.drag.keyboard != keyboard::Hold::Idle {
            if let Some(key) = keyboard::Key::from_name(&event.keystroke.key)
                && self.keyboard_key(None, key, window, cx)
            {
                cx.stop_propagation();
            }
            return;
        }
        if event.keystroke.key == "tab" && !modal {
            if event.keystroke.modifiers.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            cx.notify();
            return;
        }
        if self.pending_column.is_some() {
            if matches!(event.keystroke.key.as_str(), "enter" | "escape") {
                self.cancel_column_hide(cx);
            }
            return;
        }
        if self.project_dialog.is_some() {
            return self.project_dialog_key(event, cx);
        }
        if self.pending_delete.is_some() {
            // Cancel is the default action, so Enter and Escape both close the dialog.
            if matches!(event.keystroke.key.as_str(), "enter" | "escape") {
                self.cancel_delete(cx);
            }
            return;
        }
        let Some(rename) = self.rename.as_mut() else {
            return;
        };
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "enter" => self.commit_rename(cx),
            "escape" => self.cancel_rename(cx),
            "backspace" => {
                rename.draft.pop();
                rename.error = None;
                cx.notify();
            }
            _ => {
                let modifiers = keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                if let Some(text) = typed_text(keystroke) {
                    rename.draft.push_str(text);
                    rename.error = None;
                    cx.notify();
                }
            }
        }
    }
}

fn store(cx: &App) -> &looji_persistence::SqliteStore {
    &cx.global::<AppDatabase>().store
}

fn rename_message(err: &DomainError) -> String {
    match err {
        DomainError::EmptyName => "Name can't be empty.".to_string(),
        DomainError::DuplicateName { name } => {
            format!("A workspace named \"{name}\" already exists.")
        }
        other => format!("{other}."),
    }
}

fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            DEFAULT_WINDOW_SIZE,
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            // Hides the native caption; `TitleBar` draws our own.
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_id: Some(APP_NAME.to_lowercase()),
        window_min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let header = match self.projects.active() {
            Some(project) => format!("{} / {}", self.list.active().name(), project.name()),
            None => self.list.active().name().to_string(),
        };
        let body = projects::content(self, &theme, cx);
        let team = self.projects.active_id().is_some().then(|| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(list_view::switch(self, &theme, cx))
                .child(team_dialog::open_button(&theme, cx))
                .into_any_element()
        });
        let delete_dialog = self
            .pending_delete
            .clone()
            .map(|pending| delete::dialog(&pending, &theme, cx).into_any_element());
        let column_dialog = self
            .pending_column
            .clone()
            .map(|pending| columns::dialog(&pending, &theme, cx));
        let project_dialog = self
            .project_dialog
            .clone()
            .map(|dialog| projects::dialog(&dialog, &theme, cx));

        div()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            // A press anywhere that isn't an editor ends the tag input. Editors stop their presses.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.blur_tag_editor(cx);
                    this.blur_search(cx);
                    this.filter_menu = None;
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.palette.surface)
            .text_color(theme.palette.text_primary)
            .child(TitleBar::new(APP_NAME))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(self, cx))
                    .child(main_pane(
                        header,
                        team,
                        body,
                        panel::render(self, &theme, cx),
                        &theme,
                    )),
            )
            .children(task_dialog::overlay(self, &theme, cx))
            .children(team_dialog::overlay(self, &theme, cx))
            .child(dragging::listeners(cx.entity().downgrade()))
            .children(delete_dialog)
            .children(project_dialog)
            .children(column_dialog)
            // Last, so the lifted card draws above everything else.
            .children(dragging::overlay(self, &theme))
    }
}

fn main_pane(
    header: String,
    trailing: Option<AnyElement>,
    body: AnyElement,
    side: Option<AnyElement>,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .h(px(40.))
                .px(theme.spacing.lg)
                .border_b_1()
                .border_color(theme.palette.border)
                .text_size(theme.typography.title)
                .child(div().flex_1().min_w_0().truncate().child(header))
                .children(trailing),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .child(div().flex_1().min_w_0().min_h_0().child(body))
                .children(side),
        )
}

/// The text a keystroke types. The space key can arrive without `key_char`, so it's
/// handled explicitly.
pub(super) fn typed_text(keystroke: &Keystroke) -> Option<&str> {
    match keystroke.key_char.as_deref() {
        Some(text) => Some(text),
        None if keystroke.key == "space" => Some(" "),
        None => None,
    }
}
