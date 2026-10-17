//! Projects in the sidebar, the board for the selected project, and the create-project dialog.

use gpui::{prelude::*, *};
use looji_domain::MAX_DESCRIPTION_CHARS;
use looji_domain::{
    DomainError, Project, ProjectId, ProjectRepository, RepoError, clean_description,
    validate_project_name,
};

use super::{
    WorkspaceView, board, board_search, filter_bar, list_view,
    project_state::{ProjectList, load_board},
    roadmap, store, typed_text, workload_view,
};
use crate::theme::Theme;

const NAME_TAKEN_MESSAGE: &str = "A project with that name already exists in this workspace.";

/// Which field of the create dialog receives typed text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Field {
    #[default]
    Name,
    Description,
}

/// The create-project dialog's state. `None` on the view means it's closed.
#[derive(Debug, Clone, Default)]
pub(super) struct ProjectDialog {
    pub name: String,
    pub description: String,
    pub field: Field,
    pub error: Option<String>,
}

impl ProjectDialog {
    fn active_text(&mut self) -> &mut String {
        match self.field {
            Field::Name => &mut self.name,
            Field::Description => &mut self.description,
        }
    }
}

impl WorkspaceView {
    /// Reloads the projects of the active workspace and selects the first one.
    pub(super) fn reload_projects(&mut self, cx: &mut Context<Self>) {
        self.reset_search();
        self.reset_list_position();
        let workspace = self.list.active_workspace_id();
        match ProjectList::for_workspace(store(cx), workspace) {
            Ok(projects) => self.projects = projects,
            Err(err) => {
                self.projects = ProjectList::empty();
                self.status = Some(format!("Could not load projects: {err}"));
            }
        }
        self.load_project_filter(cx);
        self.load_list_settings(cx);
        self.load_project_view(cx);
        self.reload_board(cx);
        self.refresh_code_source(cx);
    }

    /// Loads the board and the team for the selected project. Nothing is loaded when no project is
    /// selected.
    pub(super) fn reload_board(&mut self, cx: &mut Context<Self>) {
        self.reload_members(cx);
        self.board = match self.projects.active_id() {
            Some(project) => load_board(store(cx), project).unwrap_or_else(|err| {
                self.status = Some(format!("Could not load the board: {err}"));
                Vec::new()
            }),
            None => Vec::new(),
        };
        self.cancel_keyboard_if_gone(cx);
        self.close_panel_if_gone(cx);
        self.close_plan_drafts_if_gone();
        self.close_tag_editors_if_gone();
        self.prune_project_filter(cx);
        self.refresh_list(cx);
        self.gather_screen_tasks(cx);
    }

    pub(super) fn select_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.save_project_view(cx);
        self.reset_search();
        self.reset_list_position();
        // Choosing a project leaves any workspace screen for its board.
        self.screen = None;
        self.projects.select(id);
        self.load_project_filter(cx);
        self.load_list_settings(cx);
        self.load_project_view(cx);
        self.reload_board(cx);
        self.refresh_code_source(cx);
        cx.notify();
    }

    pub(super) fn open_project_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_rename(cx);
        self.project_dialog = Some(ProjectDialog::default());
        self.focus_handle.focus(window);
        cx.notify();
    }

    pub(super) fn cancel_project_dialog(&mut self, cx: &mut Context<Self>) {
        if self.project_dialog.take().is_some() {
            cx.notify();
        }
    }

    /// Validates the form, then saves. Nothing is written if validation fails. A save runs
    /// synchronously on the UI thread, so a second submit can't start while one is in progress.
    pub(super) fn submit_project(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.project_dialog.clone() else {
            return;
        };
        let workspace = self.list.active_workspace_id();

        let name = match validate_project_name(&dialog.name, self.projects.items(), None) {
            Ok(name) => name,
            Err(err) => return self.reject_project(project_message(&err), cx),
        };
        let description = match clean_description(Some(&dialog.description)) {
            Ok(description) => description,
            Err(err) => return self.reject_project(project_message(&err), cx),
        };

        let mut project = match Project::new(workspace, &name) {
            Ok(project) => project,
            Err(err) => return self.reject_project(project_message(&err), cx),
        };
        if let Err(err) = project.set_description(description.as_deref()) {
            return self.reject_project(project_message(&err), cx);
        }

        match ProjectRepository::save(store(cx), &project) {
            Ok(()) => {
                self.projects.add_and_select(project);
                self.project_dialog = None;
                self.reload_board(cx);
            }
            Err(RepoError::NameTaken) => self.reject_project(NAME_TAKEN_MESSAGE.into(), cx),
            Err(err) => self.reject_project(format!("Could not create the project: {err}"), cx),
        }
        cx.notify();
    }

    fn reject_project(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.project_dialog.as_mut() {
            dialog.error = Some(message);
        }
        cx.notify();
    }

    /// Keyboard input for the open dialog. Enter submits, Escape cancels, Tab switches fields.
    pub(super) fn project_dialog_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_project_dialog(cx),
            "enter" => self.submit_project(cx),
            "tab" => {
                if let Some(dialog) = self.project_dialog.as_mut() {
                    dialog.field = match dialog.field {
                        Field::Name => Field::Description,
                        Field::Description => Field::Name,
                    };
                }
                cx.notify();
            }
            "backspace" => {
                if let Some(dialog) = self.project_dialog.as_mut() {
                    dialog.active_text().pop();
                    dialog.error = None;
                }
                cx.notify();
            }
            _ => {
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                if let (Some(dialog), Some(text)) =
                    (self.project_dialog.as_mut(), typed_text(&event.keystroke))
                {
                    dialog.active_text().push_str(text);
                    dialog.error = None;
                    cx.notify();
                }
            }
        }
    }

    fn focus_project_field(&mut self, field: Field, cx: &mut Context<Self>) {
        if let Some(dialog) = self.project_dialog.as_mut() {
            dialog.field = field;
        }
        cx.notify();
    }
}

fn project_message(err: &DomainError) -> String {
    match err {
        DomainError::EmptyName => "Enter a project name.".to_string(),
        DomainError::DuplicateProjectName { name } => {
            format!("A project named \"{name}\" already exists in this workspace.")
        }
        DomainError::DescriptionTooLong { max } => {
            format!("The description can be at most {max} characters.")
        }
        other => format!("{other}."),
    }
}

/// The projects section of the sidebar, under the workspace list.
pub(super) fn sidebar_section(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    // While a workspace screen is open, no project row is highlighted.
    let active = match view.screen {
        Some(_) => None,
        None => view.projects.active_id(),
    };
    let rows: Vec<AnyElement> = view
        .projects
        .items()
        .iter()
        .map(|project| {
            project_row(project, active == Some(project.id()), theme, cx).into_any_element()
        })
        .collect();
    let is_empty = view.projects.items().is_empty();

    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .mt(theme.spacing.md)
        .child(
            div()
                .px(theme.spacing.sm)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("Projects"),
        )
        .children(rows)
        .when(is_empty, |section| {
            section.child(
                div()
                    .px(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.text_secondary)
                    .child("No projects yet"),
            )
        })
        .child(
            crate::components::nav_item("new-project", "plus", "New project", false, None, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_project_dialog(window, cx);
                })),
        )
}

fn project_row(
    project: &Project,
    is_active: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = project.id();
    div()
        .id(id.as_uuid())
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .when(is_active, |row| {
            row.bg(theme.palette.surface_selected)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!is_active, |row| {
            row.hover(|style| style.bg(theme.palette.surface_hover))
        })
        // A dot marks the open project, as in the design's sidebar.
        .child(crate::components::dot(
            if is_active {
                theme.palette.accent
            } else {
                theme.palette.border_control
            },
            7.,
        ))
        .child(div().min_w_0().truncate().child(project.name().to_string()))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select_project(id, cx)))
}

/// The main pane's body: the selected project's board, or an empty state with a way to start.
pub(super) fn content(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    #[cfg(debug_assertions)]
    if std::env::var_os("LOOJI_CARD_GALLERY").is_some() {
        return super::card::gallery(theme).into_any_element();
    }
    match view.projects.active() {
        None => empty_workspace(theme, cx).into_any_element(),
        Some(project) => {
            let filters = filter_bar::bar(view, theme, cx);
            // The board and the list read the same query, so the counts in the bar agree with both.
            let (column, shown, notice) = match view.view_mode {
                list_view::ViewMode::Board => {
                    let cards = board::card_context(view);
                    let shown = board_search::shown_tasks(view);
                    let notice = filter_bar::empty_notice(view, shown.len(), theme, cx);
                    let column = board::render(project, &shown, &cards, &view.drag, theme, cx)
                        .into_any_element();
                    (column, shown.len(), notice)
                }
                list_view::ViewMode::Workload => (
                    workload_view::render(view, theme, cx),
                    board_search::shown_tasks(view).len(),
                    None,
                ),
                list_view::ViewMode::Roadmap => (
                    roadmap::render(view, theme, cx),
                    board_search::shown_tasks(view).len(),
                    None,
                ),
                list_view::ViewMode::List => (
                    list_view::render(view, theme, cx),
                    view.task_list.total as usize,
                    None,
                ),
            };
            let search = board_search::bar(view, shown, view.board.len(), theme, cx);
            div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(filters)
                .child(search)
                .children(notice)
                .child(
                    // A flex column, so the view inside fills the height left under the bars. A plain wrapper would let the
                    // board grow to the height of all its cards instead of scrolling inside its columns.
                    div().flex().flex_col().flex_1().min_h_0().child(column),
                )
                .into_any_element()
        }
    }
}

fn empty_workspace(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(theme.spacing.md)
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(340.))
                .child(crate::components::icon_tile("layout-kanban", theme))
                .child(
                    div()
                        .mt(theme.spacing.md)
                        .text_size(theme.typography.heading)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Start your first project"),
                )
                .child(
                    div()
                        .mt(theme.spacing.xs)
                        .text_color(theme.palette.text_secondary)
                        .line_height(relative(1.6))
                        .child(
                            "Projects hold your tasks and a board to track them, plus planning drafts for a whole plan.",
                        ),
                )
                .child(
                    div().mt(theme.spacing.lg).child(
                        crate::components::button(
                            "create-project-cta",
                            "Create project",
                            crate::components::Variant::Primary,
                            theme,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_project_dialog(window, cx);
                        })),
                    ),
                )
                .child(
                    div()
                        .mt(theme.spacing.lg)
                        .pt(theme.spacing.xs)
                        .border_t_1()
                        .border_color(theme.palette.border)
                        .child(crate::components::numbered_step(
                            1,
                            "Name a project",
                            "Something you're working toward.",
                            theme,
                        ))
                        .child(crate::components::numbered_step(
                            2,
                            "Add tasks",
                            "Each one goes on the board, in To do.",
                            theme,
                        ))
                        .child(crate::components::numbered_step(
                            3,
                            "Sketch a plan in a draft",
                            "Think it through, then turn it into tasks.",
                            theme,
                        )),
                ),
        )
}

/// The create-project dialog, drawn over the window.
pub(super) fn dialog(
    dialog: &ProjectDialog,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let description_count = dialog.description.chars().count();

    div()
        .id("project-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_project_dialog(cx)))
        .child(
            div()
                .id("project-panel")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .w(px(420.))
                .p(theme.spacing.lg)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .shadow(theme.elevation.overlay.clone())
                .border_1()
                .border_color(theme.palette.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(div().text_size(theme.typography.title).child("New project"))
                .child(field(
                    "Name",
                    &dialog.name,
                    dialog.field == Field::Name,
                    None,
                    Field::Name,
                    theme,
                    cx,
                ))
                .child(field(
                    "Description (optional)",
                    &dialog.description,
                    dialog.field == Field::Description,
                    Some(format!("{description_count}/{MAX_DESCRIPTION_CHARS}")),
                    Field::Description,
                    theme,
                    cx,
                ))
                .when_some(dialog.error.as_ref(), |panel, message| {
                    panel.child(
                        div()
                            .text_size(theme.typography.body)
                            .text_color(theme.palette.danger)
                            .child(message.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(
                            crate::components::button(
                                "project-cancel",
                                "Cancel",
                                crate::components::Variant::Ghost,
                                theme,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| this.cancel_project_dialog(cx),
                            )),
                        )
                        .child(
                            div()
                                .id("project-create")
                                .px(theme.spacing.md)
                                .py(theme.spacing.xs)
                                .rounded(theme.radius.md)
                                .cursor_pointer()
                                .bg(theme.palette.accent)
                                .text_color(theme.palette.on_accent)
                                .child("Create")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.submit_project(cx)
                                })),
                        ),
                ),
        )
        .into_any_element()
}

fn field(
    label: &'static str,
    text: &str,
    active: bool,
    counter: Option<String>,
    which: Field,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let caret = if active { "|" } else { "" };
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(label)
                .when_some(counter, |row, count| row.child(count)),
        )
        .child(
            div()
                .id(label)
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.sm)
                .border_1()
                .border_color(if active {
                    theme.palette.accent
                } else {
                    theme.palette.border_control
                })
                .bg(theme.palette.surface)
                .child(format!("{text}{caret}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.focus_project_field(which, cx)
                })),
        )
}
