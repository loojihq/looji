//! Projects in the sidebar, the board for the selected project, and the create-project dialog.

use gpui::{prelude::*, *};
use looji_domain::MAX_DESCRIPTION_CHARS;
use looji_domain::{
    DomainError, Project, ProjectId, ProjectRepository, RepoError, Task, clean_description,
    validate_project_name,
};

use super::{
    WorkspaceView,
    project_state::{ProjectList, load_board},
    store, typed_text,
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
        let workspace = self.list.active_workspace_id();
        match ProjectList::for_workspace(store(cx), workspace) {
            Ok(projects) => self.projects = projects,
            Err(err) => {
                self.projects = ProjectList::empty();
                self.status = Some(format!("Could not load projects: {err}"));
            }
        }
        self.reload_board(cx);
    }

    /// Loads the board for the selected project. Nothing is loaded when no project is selected.
    fn reload_board(&mut self, cx: &mut Context<Self>) {
        self.board = match self.projects.active_id() {
            Some(project) => load_board(store(cx), project).unwrap_or_else(|err| {
                self.status = Some(format!("Could not load the board: {err}"));
                Vec::new()
            }),
            None => Vec::new(),
        };
    }

    pub(super) fn select_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.projects.select(id);
        self.reload_board(cx);
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
    let active = view.projects.active_id();
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
        .gap_1()
        .mt_3()
        .child(
            div()
                .px_2()
                .text_xs()
                .text_color(theme.text.opacity(0.6))
                .child("PROJECTS"),
        )
        .children(rows)
        .when(is_empty, |section| {
            section.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(theme.text.opacity(0.6))
                    .child("No projects yet"),
            )
        })
        .child(
            div()
                .id("new-project")
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(theme.ghost_element_hover))
                .child("+ New project")
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
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .truncate()
        .when(is_active, |row| row.bg(theme.element_selected))
        .when(!is_active, |row| {
            row.hover(|style| style.bg(theme.ghost_element_hover))
        })
        .child(project.name().to_string())
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select_project(id, cx)))
}

/// The main pane's body: the selected project's board, or an empty state with a way to start.
pub(super) fn content(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    match view.projects.active() {
        None => empty_workspace(theme, cx).into_any_element(),
        Some(project) => board(project, &view.board, theme).into_any_element(),
    }
}

fn empty_workspace(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_color(theme.text.opacity(0.7))
                .child("This workspace has no projects yet."),
        )
        .child(
            div()
                .id("create-project-cta")
                .px_3()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .border_1()
                .border_color(theme.border)
                .hover(|style| style.bg(theme.ghost_element_hover))
                .child("Create project")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_project_dialog(window, cx);
                })),
        )
}

fn board(project: &Project, tasks: &[Task], theme: &Theme) -> impl IntoElement {
    let body: AnyElement = if tasks.is_empty() {
        div()
            .text_color(theme.text.opacity(0.7))
            .child("No tasks yet.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(tasks.iter().map(|task| {
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.panel_background)
                    .child(task.title().to_string())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text.opacity(0.6))
                            .child(task.status().as_str().to_string()),
                    )
            }))
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .p_4()
        .gap_3()
        .child(div().text_lg().child(project.name().to_string()))
        .when_some(project.description(), |header, text| {
            header.child(
                div()
                    .text_sm()
                    .text_color(theme.text.opacity(0.7))
                    .child(text.to_string()),
            )
        })
        .child(body)
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
        .bg(gpui::black().opacity(0.5))
        .occlude()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_project_dialog(cx)))
        .child(
            div()
                .id("project-panel")
                .flex()
                .flex_col()
                .gap_3()
                .w(px(420.))
                .p_4()
                .rounded_lg()
                .bg(theme.panel_background)
                .border_1()
                .border_color(theme.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(div().text_lg().child("New project"))
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
                            .text_sm()
                            .text_color(theme.error)
                            .child(message.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(
                            div()
                                .id("project-cancel")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .border_1()
                                .border_color(theme.border)
                                .hover(|style| style.bg(theme.ghost_element_hover))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cancel_project_dialog(cx)
                                })),
                        )
                        .child(
                            div()
                                .id("project-create")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(theme.element_selected)
                                .hover(|style| style.opacity(0.85))
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
        .gap_1()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_xs()
                .text_color(theme.text.opacity(0.7))
                .child(label)
                .when_some(counter, |row, count| row.child(count)),
        )
        .child(
            div()
                .id(label)
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(if active { theme.text } else { theme.border })
                .bg(theme.editor_background)
                .child(format!("{text}{caret}"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.focus_project_field(which, cx)
                })),
        )
}
