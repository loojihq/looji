//! The main window: a title bar over a sidebar of workspaces and projects, and the main pane.

mod avatar;
mod board;
mod columns;
mod delete;
mod preferences_panel;
mod project_state;
mod projects;
mod sidebar;
mod state;

use delete::PendingDelete;
use project_state::ProjectList;
use projects::ProjectDialog;

use gpui::{prelude::*, *};
use looji_domain::{
    DomainError, RepoError, SettingsRepository, Task, Workspace, WorkspaceId, WorkspaceRepository,
    next_untitled_name, validate_name,
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
    /// The create-project dialog, while it's open.
    project_dialog: Option<ProjectDialog>,
    /// A column hide waiting for confirmation.
    pending_column: Option<columns::PendingColumn>,
}

impl WorkspaceView {
    pub fn open_window(cx: &mut App, list: WorkspaceList) -> Result<WindowHandle<Self>> {
        cx.open_window(window_options(cx), |_, cx| {
            cx.new(|cx| {
                let mut view = Self {
                    focus_handle: cx.focus_handle(),
                    list,
                    rename: None,
                    status: None,
                    pending_delete: None,
                    projects: ProjectList::empty(),
                    board: Vec::new(),
                    project_dialog: None,
                    pending_column: None,
                };
                view.reload_projects(cx);
                view
            })
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

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
                    .child(main_pane(header, body, &theme)),
            )
            .children(delete_dialog)
            .children(project_dialog)
            .children(column_dialog)
    }
}

fn main_pane(header: String, body: AnyElement, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .h(px(40.))
                .px(theme.spacing.lg)
                .border_b_1()
                .border_color(theme.palette.border)
                .text_size(theme.typography.title)
                .child(header),
        )
        .child(body)
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
