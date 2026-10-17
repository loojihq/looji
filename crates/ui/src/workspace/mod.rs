//! The main window: a title bar over a sidebar of workspaces and projects, and the main pane.

mod autoscroll;
mod avatar;
mod board;
mod board_search;
mod card;
mod chat;
mod chat_render;
mod columns;
mod date_picker;
mod delete;
mod dialog_window;
mod drag;
mod dragging;
mod export;
mod fields;
mod filter_bar;
mod keyboard;
mod landing;
mod layout;
mod list_view;
mod markdown;
mod members;
mod moves;
mod palette;
mod panel;
mod plan_drafts;
mod plan_refine;
mod plan_review;
mod plan_wizard;
mod preferences_panel;
mod profile;
mod project_state;
mod project_views;
mod projects;
mod repo_dialog;
mod roadmap;
mod roadmap_layout;
mod schedule_dialog;
mod screen;
mod search;
mod sidebar;
mod state;
mod switcher;
mod task_dialog;
mod task_edit;
mod task_editors;
mod task_form;
mod team_dialog;
mod tour;
mod updates;
mod workload_view;
mod workspace_setup;

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
    title_bar::{SidebarHead, TitleBar},
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
    /// The task chat, while it is open.
    chat_dialog: Option<chat::ChatDialog>,
    /// Follows the newest message in the chat, so a streaming reply stays in view.
    chat_scroll: ScrollHandle,
    /// The check for a newer release, running in the background, and the notice it produced, if any.
    update_task: Option<gpui::Task<()>>,
    update_notice: Option<looji_updates::UpdateStatus>,
    update_install: Option<updates::Install>,
    update_worker: Option<gpui::Task<()>>,
    /// The first-run tour, while it is showing.
    tour: Option<looji_domain::Tour>,
    /// The repository dialog, while it's open, and the status of the selected project's folder. A check
    /// numbered `code_check_seq` is the only one allowed to replace the status it set.
    repo_dialog: Option<repo_dialog::RepoDialog>,
    code_source: Option<repo_dialog::Source>,
    code_check_seq: u64,
    /// The working hours dialog, while it's open, and one focus handle for each of its controls.
    schedule_dialog: Option<schedule_dialog::ScheduleDialog>,
    schedule_focus: Vec<FocusHandle>,
    /// The planning wizard, while it's open, and one focus handle for each of its controls.
    plan_wizard: Option<plan_wizard::PlanWizardDialog>,
    /// The planning drafts of the selected project, while their list is open.
    plan_drafts: Option<plan_drafts::PlanDraftsDialog>,
    plan_focus: Vec<FocusHandle>,
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
    /// The sort and the shown columns of the selected project's list, and whether the column picker is open.
    list_settings: looji_domain::ListView,
    list_columns_open: bool,
    /// The roadmap's scale, grouping, and the day it is centred on.
    roadmap: roadmap::RoadmapState,
    /// The workload view's period and whether done tasks count.
    workload: workload_view::WorkloadState,
    /// Each project's view state, as it was loaded, and the active project's scroll positions. The positions
    /// are read from the live handles below when the user leaves a view.
    loaded_view_state: looji_domain::ProjectViewState,
    view_scroll: std::collections::BTreeMap<String, looji_domain::ScrollOffset>,
    /// Set when a view was entered or a project loaded, so its scroll positions are put back on the next frame.
    pending_scroll: bool,
    scroll_list: ScrollHandle,
    scroll_roadmap: ScrollHandle,
    scroll_workload: ScrollHandle,
    /// One focus handle per view option in the switcher, in order.
    view_focus: Vec<FocusHandle>,
    /// Saves the active project when the app quits.
    quit_subscription: Option<Subscription>,
    /// The board filter of the selected project, and the facet menu that is open, if any.
    filter: TaskFilter,
    filter_menu: Option<filter_bar::FilterMenu>,
    /// One focus handle for each control in the filter bar, in tab order.
    filter_focus: Vec<FocusHandle>,
    /// The command palette, while it's open.
    palette: Option<palette::PaletteState>,
    /// The workspace screen in place of a board, if one is open, and the tasks it gathered.
    screen: Option<screen::Screen>,
    screen_entries: Vec<screen::Entry>,
    /// Whether the workspace switcher's menu is open.
    workspace_menu: bool,
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
                    chat_dialog: None,
                    chat_scroll: ScrollHandle::new(),
                    update_task: None,
                    update_notice: None,
                    update_install: None,
                    update_worker: None,
                    tour: None,
                    repo_dialog: None,
                    code_source: None,
                    code_check_seq: 0,
                    schedule_dialog: None,
                    plan_wizard: None,
                    plan_drafts: None,
                    plan_focus: (0..plan_wizard::CONTROLS.len())
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                    schedule_focus: (0..schedule_dialog::CONTROL_COUNT)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                    panel: None,
                    tag_edit: None,
                    card_editing: None,
                    search_draft: String::new(),
                    search_query: String::new(),
                    search_focused: false,
                    search_debounce: search::Debounce::default(),
                    view_mode: list_view::ViewMode::default(),
                    task_list: list_view::ListState::default(),
                    list_settings: looji_domain::ListView::default(),
                    list_columns_open: false,
                    roadmap: roadmap::RoadmapState::default(),
                    workload: workload_view::WorkloadState::default(),
                    loaded_view_state: looji_domain::ProjectViewState::default(),
                    view_scroll: std::collections::BTreeMap::new(),
                    pending_scroll: false,
                    scroll_list: ScrollHandle::new(),
                    scroll_roadmap: ScrollHandle::new(),
                    scroll_workload: ScrollHandle::new(),
                    view_focus: (0..looji_domain::ProjectView::ALL.len())
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                    quit_subscription: None,
                    filter: TaskFilter::default(),
                    filter_menu: None,
                    palette: None,
                    screen: None,
                    screen_entries: Vec::new(),
                    workspace_menu: false,
                    filter_focus: (0..filter_bar::CONTROL_COUNT)
                        .map(|_| cx.focus_handle().tab_stop(true))
                        .collect(),
                };
                view.reload_projects(cx);
                view.gather_screen_tasks(cx);
                view.start_update_checks(cx);
                view.start_tour_if_new(cx);
                view
            });
            // Focus starts on the workspace, so keys reach it, including Tab for the board.
            let focus = view.read(cx).focus_handle.clone();
            focus.focus(window);
            // The active project's view is saved when the app quits, after the last window has gone.
            let weak = view.downgrade();
            let quit = cx.on_app_quit(move |cx| {
                let _ = weak.update(cx, |view, cx| view.save_on_quit(cx));
                async {}
            });
            view.update(cx, |view, _| view.quit_subscription = Some(quit));
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
                // The project being left keeps its view and scroll, saved before the switch.
                self.save_project_view(cx);
                self.list.set_active(id);
                // A screen belongs to the workspace it was opened in.
                self.screen = None;
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
                // Make the new workspace the active one first, so its name is the one being edited.
                self.switch_to(id, cx);
                self.begin_rename(id, window, cx);
                // The new workspace's own screen holds the name field and the next steps.
                self.open_screen(screen::Screen::Workspace, cx);
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
        // The tour is on top of everything, so it takes the keys first.
        if self.tour.is_some() {
            return self.tour_key(event, cx);
        }
        if self.workspace_menu && event.keystroke.key == "escape" {
            return self.close_workspace_menu(cx);
        }
        // The palette takes every key while it's open, so typing doesn't reach the board.
        if self.palette.is_some() {
            return self.palette_key(event, window, cx);
        }
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
        if event.keystroke.key == "k" && (modifiers.control || modifiers.platform) {
            return self.open_palette(cx);
        }
        if self.key_pressed(event, cx) {
            return;
        }
        if self.task_dialog.is_some() {
            return self.task_dialog_key(event, cx);
        }
        if self.chat_dialog.is_some() {
            return self.chat_key(event, cx);
        }
        if self.plan_drafts.is_some() && event.keystroke.key == "escape" {
            return self.close_plan_drafts(cx);
        }
        if self.plan_wizard.is_some() {
            return self.plan_wizard_key(event, window, cx);
        }
        if self.schedule_dialog.is_some() {
            return self.schedule_dialog_key(event, window, cx);
        }
        if self.repo_dialog.is_some() {
            return self.repo_dialog_key(event, cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _frame = profile::frame(window);
        if std::mem::take(&mut self.pending_scroll) {
            self.schedule_scroll_restore(window);
        }
        let theme = cx.theme().clone();
        let header = project_header(self, &theme, cx);
        let body = match self.screen {
            Some(screen) => screen::render(self, screen, &theme, window, cx),
            None => projects::content(self, &theme, cx),
        };

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
                    this.close_task_menus(cx);
                    this.filter_menu = None;
                    this.list_columns_open = false;
                    this.close_workspace_menu(cx);
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.palette.surface)
            .text_color(theme.palette.text_primary)
            // Windows drags the window from this strip over the top of the window, as JLC does. It is
            // painted first, so the title bar's controls and the bars' own elements sit above it.
            .when(cfg!(target_os = "windows"), |root| {
                root.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(crate::title_bar::TITLE_BAR_HEIGHT)
                        .window_control_area(WindowControlArea::Drag),
                )
            })
            // The sidebar runs the full height, and the title bar covers only the main pane.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .h_full()
                            .child(SidebarHead::new(APP_NAME, sidebar::SIDEBAR_WIDTH))
                            .child(sidebar::render(self, cx)),
                    )
                    .child(main_pane(header, body, panel::render(self, &theme, cx))),
            )
            .children(palette::overlay(self, &theme, cx))
            .children(updates::notice(self, &theme, cx))
            .children(tour::overlay(self, &theme, cx))
            .child(dragging::listeners(cx.entity().downgrade()))
            // Last, so the lifted card draws above everything else.
            .children(dragging::overlay(self, &theme))
    }
}

/// The top of the main pane: a breadcrumb and title, the project's counts, New task, and the view
/// tabs with the project's other actions on the right of the same line.
fn project_header(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    // A workspace screen has its own title, and no project tabs or actions.
    if let Some(screen) = view.screen {
        return screen::header(view, screen, theme);
    }
    let workspace = view.list.active().name().to_string();
    let project = view.projects.active();

    let crumb = div()
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .truncate()
        .child(format!("{workspace} / Projects"));

    let Some(project) = project else {
        // No project yet: the workspace's name, and nothing to act on.
        return div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(theme.spacing.lg)
            .pt(theme.spacing.md)
            .child(crumb)
            .child(
                div()
                    .mt(theme.spacing.xs)
                    .text_size(theme.typography.heading)
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(workspace),
            )
            .into_any_element();
    };

    let total = view.board.len();
    let done = view
        .board
        .iter()
        .filter(|task| task.status() == looji_domain::Status::Done)
        .count();
    let open = total - done;

    // The card: the project's name and counts, the actions, and how far through its tasks it is.
    let card = div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.border)
        .bg(theme.palette.surface_raised)
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(theme.spacing.md)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(theme.typography.heading)
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(project.name().to_string()),
                        )
                        .child(
                            div()
                                .mt(px(2.))
                                .text_size(theme.typography.body)
                                .text_color(theme.palette.text_secondary)
                                .child(format!(
                                    "{total} {} · {open} open",
                                    if total == 1 { "task" } else { "tasks" }
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(plan_wizard::open_button(theme, cx))
                        .child(export::open_button(theme, cx))
                        .child(board::columns_button(project, theme, cx))
                        .child(board::new_task_button(view, theme, cx)),
                ),
        )
        .child(progress(done, total, theme));

    // The views share one row: the segmented switch on the left, and the project's other windows on the right.
    let views = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .child(list_view::switch(view, theme, cx))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(plan_drafts::open_button(theme, cx))
                .child(schedule_dialog::open_button(theme, cx))
                .child(repo_dialog::open_button(theme, cx)),
        );

    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .flex_shrink_0()
        .px(theme.spacing.lg)
        .pt(theme.spacing.md)
        .child(crumb)
        .child(card)
        // The new-task form opens right under the card, where the New task button is.
        .children(task_dialog::new_task_form(view, theme, cx).map(|form| div().child(form)))
        .child(views)
        .into_any_element()
}

/// How far through the project's tasks it is: a bar filled to the share of done tasks, with the count.
fn progress(done: usize, total: usize, theme: &Theme) -> impl IntoElement {
    let fraction = if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    };
    let percent = (fraction * 100.0).round() as u32;
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .w_full()
                .h(px(6.))
                .rounded(px(3.))
                .bg(theme.palette.border)
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction))
                        .rounded(px(3.))
                        .bg(theme.palette.accent),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("{done} of {total} done"))
                .child(format!("{percent}%")),
        )
}

fn main_pane(header: AnyElement, body: AnyElement, side: Option<AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(TitleBar)
        .child(header)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .child(
                    // A flex column, so the board inside it fills the height and its columns scroll. Without it the board
                    // takes the height of all its cards and runs off the bottom of the window.
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(body),
                )
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
