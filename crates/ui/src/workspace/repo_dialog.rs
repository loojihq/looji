//! The repository dialog: the local folder a project is connected to. It opens from the Repository button in the
//! header and owns the keyboard while it's open.
//!
//! Folders are checked on a background thread, since a network drive can block. While a check runs the badge says
//! "Checking". A folder is saved only after it passes the check. Only Disconnect removes the saved path. A folder
//! that later disappears shows as Broken, and its path is kept so it can be located again.

use std::path::Path;

use gpui::{prelude::*, *};
use looji_domain::{
    CodeSourceError, CodeSourceRepository, CodeSourceStatus, PathCheck, ProjectId,
    clean_path_input, status_for,
};
use looji_persistence::repo_folder::check_repo_folder;

use super::fields::text_box;
use super::task_dialog::button;
use super::team_dialog::error_text;
use super::{WorkspaceView, store, typed_text};
use crate::theme::Theme;

/// What the dialog shows for the project's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Shown {
    Checking,
    Known(CodeSourceStatus),
    /// The saved path couldn't be read from the database.
    Failed(String),
}

/// The folder status of the selected project. `check` numbers each check, so a result that arrives late can't
/// replace a newer one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Source {
    pub project: ProjectId,
    pub check: u64,
    pub shown: Shown,
}

/// An open repository dialog: the path typed or browsed to, and the message for the last attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepoDialog {
    pub project: ProjectId,
    pub draft: String,
    pub error: Option<String>,
}

impl RepoDialog {
    fn new(project: ProjectId) -> Self {
        Self {
            project,
            draft: String::new(),
            error: None,
        }
    }
}

impl WorkspaceView {
    /// Opens the dialog for the selected project and checks its folder again, so the badge reflects the disk now.
    pub(super) fn open_repo_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            return;
        };
        self.repo_dialog = Some(RepoDialog::new(project));
        self.refresh_code_source(cx);
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn close_repo_dialog(&mut self, cx: &mut Context<Self>) {
        self.repo_dialog = None;
        cx.notify();
    }

    /// Reads the selected project's saved path and checks it. Called when a project is selected or the project
    /// list is reloaded, so the status matches the folder on disk at that moment.
    pub(super) fn refresh_code_source(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active_id() else {
            self.code_source = None;
            return cx.notify();
        };
        match CodeSourceRepository::load_path(store(cx), project) {
            Ok(path) => self.check_code_source(project, path, cx),
            Err(err) => {
                let check = self.next_check();
                self.code_source = Some(Source {
                    project,
                    check,
                    shown: Shown::Failed(format!("Could not read the saved folder: {err}")),
                });
                cx.notify();
            }
        }
    }

    /// Checks the saved folder off the UI thread. With no saved folder there is nothing to check.
    fn check_code_source(
        &mut self,
        project: ProjectId,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let check = self.next_check();
        let Some(folder) = path else {
            self.code_source = Some(Source {
                project,
                check,
                shown: Shown::Known(CodeSourceStatus::NotConnected),
            });
            return cx.notify();
        };
        self.code_source = Some(Source {
            project,
            check,
            shown: Shown::Checking,
        });
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async move {
                    let outcome = check_repo_folder(Path::new(&folder));
                    status_for(Some(folder), outcome)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_code_check(project, check, Shown::Known(status), cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_code_check(
        &mut self,
        project: ProjectId,
        check: u64,
        shown: Shown,
        cx: &mut Context<Self>,
    ) {
        if let Some(source) = self
            .code_source
            .as_mut()
            .filter(|source| source.project == project && source.check == check)
        {
            source.shown = shown;
        }
        cx.notify();
    }

    fn next_check(&mut self) -> u64 {
        self.code_check_seq += 1;
        self.code_check_seq
    }

    /// Asks for a folder, then connects it. A cancelled picker changes nothing.
    pub(super) fn choose_repo_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = match folder.to_str() {
                Some(text) => {
                    let text = text.to_owned();
                    this.update(cx, |view, cx| view.connect_folder(text, cx))
                }
                None => this.update(cx, |view, cx| {
                    view.set_repo_error(
                        "That folder's path can't be saved. Choose another folder.".to_string(),
                        cx,
                    )
                }),
            };
        })
        .detach();
    }

    /// Checks the typed or browsed path, then saves it only if it's a git repository. A failed check leaves the
    /// saved path as it was, and the message says why.
    fn connect_folder(&mut self, typed: String, cx: &mut Context<Self>) {
        let Some(project) = self.repo_dialog.as_ref().map(|dialog| dialog.project) else {
            return;
        };
        let path = match clean_path_input(&typed) {
            Ok(path) => path,
            Err(err @ CodeSourceError::EmptyPath) => {
                return self.set_repo_error(err.to_string(), cx);
            }
        };
        let check = self.next_check();
        self.code_source = Some(Source {
            project,
            check,
            shown: Shown::Checking,
        });
        if let Some(dialog) = self.repo_dialog.as_mut() {
            dialog.draft = path.clone();
            dialog.error = None;
        }
        let folder = path.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { check_repo_folder(Path::new(&folder)) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_connect(project, check, path, outcome, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_connect(
        &mut self,
        project: ProjectId,
        check: u64,
        path: String,
        outcome: Result<(), PathCheck>,
        cx: &mut Context<Self>,
    ) {
        // A newer check has started, or the project changed, so this result is out of date.
        let current = self
            .code_source
            .as_ref()
            .is_some_and(|source| source.project == project && source.check == check);
        if !current {
            return;
        }
        match outcome {
            Ok(()) => match CodeSourceRepository::save_path(store(cx), project, &path) {
                Ok(()) => {
                    self.code_source = Some(Source {
                        project,
                        check,
                        shown: Shown::Known(CodeSourceStatus::Connected { path }),
                    });
                    if let Some(dialog) = self.repo_dialog.as_mut() {
                        dialog.draft.clear();
                        dialog.error = None;
                    }
                }
                Err(err) => {
                    eprintln!("could not save a repository folder: {err}");
                    self.set_repo_error("Could not save that folder. Try again.".to_string(), cx);
                    self.refresh_code_source(cx);
                }
            },
            Err(reason) => {
                self.set_repo_error(reason.to_string(), cx);
                self.refresh_code_source(cx);
            }
        }
        cx.notify();
    }

    /// Removes the saved path. Only this removes it, so a folder that goes missing never loses its path on its own.
    fn disconnect_folder(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.repo_dialog.as_ref().map(|dialog| dialog.project) else {
            return;
        };
        match CodeSourceRepository::clear_path(store(cx), project) {
            Ok(()) => {
                let check = self.next_check();
                self.code_source = Some(Source {
                    project,
                    check,
                    shown: Shown::Known(CodeSourceStatus::NotConnected),
                });
                if let Some(dialog) = self.repo_dialog.as_mut() {
                    dialog.draft.clear();
                    dialog.error = None;
                }
            }
            Err(err) => {
                eprintln!("could not disconnect a repository folder: {err}");
                self.set_repo_error(
                    "Could not disconnect. The folder is still connected.".to_string(),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn set_repo_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.repo_dialog.as_mut() {
            dialog.error = Some(message);
        }
        cx.notify();
    }

    /// Keys while the dialog is open. Enter connects what's typed, and Escape closes the dialog.
    pub(super) fn repo_dialog_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let modifier = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        match event.keystroke.key.as_str() {
            "escape" => self.close_repo_dialog(cx),
            "enter" => {
                if let Some(draft) = self.repo_dialog.as_ref().map(|dialog| dialog.draft.clone()) {
                    self.connect_folder(draft, cx);
                }
            }
            "backspace" => {
                if let Some(dialog) = self.repo_dialog.as_mut() {
                    dialog.draft.pop();
                    dialog.error = None;
                    cx.notify();
                }
            }
            _ => {
                if modifier {
                    return;
                }
                if let Some(text) = typed_text(&event.keystroke)
                    && let Some(dialog) = self.repo_dialog.as_mut()
                {
                    dialog.draft.push_str(text);
                    dialog.error = None;
                    cx.notify();
                }
            }
        }
    }
}

/// The repository button in the header. It's drawn only while a project is selected.
pub(super) fn open_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("repo-open")
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.palette.border_control)
        .text_size(theme.typography.body)
        .cursor_pointer()
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("Repository")
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_repo_dialog(window, cx)))
}

/// The repository dialog, drawn over the window. Nothing is drawn when it isn't open.
pub(super) fn overlay(
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let dialog = view.repo_dialog.as_ref()?;
    let project = view
        .projects
        .active()
        .map(|project| project.name().to_string())
        .unwrap_or_default();
    let shown = match &view.code_source {
        Some(source) if source.project == dialog.project => source.shown.clone(),
        _ => Shown::Checking,
    };
    let stored = match &shown {
        Shown::Known(CodeSourceStatus::Connected { path })
        | Shown::Known(CodeSourceStatus::Broken { path, .. }) => Some(path.clone()),
        _ => None,
    };
    let broken = matches!(shown, Shown::Known(CodeSourceStatus::Broken { .. }));

    let mut actions = div().flex().flex_row().gap(theme.spacing.sm).justify_end();
    if stored.is_some() {
        actions = actions.child(
            button("repo-disconnect", "Disconnect", false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.disconnect_folder(cx))),
        );
    }
    let browse_label = if broken { "Locate again" } else { "Browse" };
    let panel = div()
        .id("repo-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w(px(460.))
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .bg(theme.palette.surface_raised)
        .shadow(theme.elevation.overlay.clone())
        .border_1()
        .border_color(theme.palette.border)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(div().text_size(theme.typography.title).child("Repository"))
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(format!("The local git repository for {project}")),
        )
        // A row keeps the badge its own width; the panel is a column, which would stretch it.
        .child(div().flex().flex_row().child(badge(&shown, theme)))
        .child(status_text(&shown, theme))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .child(
                    div()
                        .id("repo-path-field")
                        .flex_1()
                        .min_w_0()
                        .child(text_box(
                            &dialog.draft,
                            "Folder path, then press Enter",
                            true,
                            theme,
                        )),
                )
                .child(
                    button("repo-connect", "Connect", true, theme).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(draft) =
                                this.repo_dialog.as_ref().map(|dialog| dialog.draft.clone())
                            {
                                this.connect_folder(draft, cx);
                            }
                        },
                    )),
                ),
        )
        .child(
            button("repo-browse", browse_label, false, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_repo_folder(cx))),
        )
        .when_some(dialog.error.clone(), |panel, message| {
            panel.child(error_text(message, theme))
        })
        .child(
            actions.child(
                button("repo-done", "Done", false, theme).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.close_repo_dialog(cx)),
                ),
            ),
        );

    Some(
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("repo-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.palette.scrim)
                    .occlude()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_repo_dialog(cx)))
                    .child(panel),
            )
            .into_any_element(),
    )
}

/// The status as a small labelled badge.
fn badge(shown: &Shown, theme: &Theme) -> Div {
    let (label, colour) = match shown {
        Shown::Checking => ("Checking", theme.palette.text_disabled),
        Shown::Known(CodeSourceStatus::Connected { .. }) => ("Connected", theme.palette.accent),
        Shown::Known(CodeSourceStatus::Broken { .. }) | Shown::Failed(_) => {
            ("Broken", theme.palette.danger)
        }
        Shown::Known(CodeSourceStatus::NotConnected) => {
            ("Not connected", theme.palette.text_secondary)
        }
    };
    div()
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .border_1()
        .border_color(colour)
        .text_color(colour)
        .text_size(theme.typography.caption)
        .child(label)
}

/// What the status means for the user: the saved path, the reason it's broken, or what to do next.
fn status_text(shown: &Shown, theme: &Theme) -> Div {
    let body = div().text_size(theme.typography.caption);
    match shown {
        Shown::Checking => body
            .text_color(theme.palette.text_secondary)
            .child("Checking the folder…"),
        Shown::Known(CodeSourceStatus::NotConnected) => body
            .text_color(theme.palette.text_secondary)
            .child("Choose the folder that holds this project's git repository."),
        Shown::Known(CodeSourceStatus::Connected { path }) => body
            .text_color(theme.palette.text_secondary)
            .child(format!("Connected to {path}")),
        Shown::Known(CodeSourceStatus::Broken { path, reason }) => body
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .child(
                div()
                    .text_color(theme.palette.text_secondary)
                    .child(format!("Saved folder: {path}")),
            )
            .child(
                div()
                    .text_color(theme.palette.danger)
                    .child(reason.to_string()),
            ),
        Shown::Failed(message) => body.text_color(theme.palette.danger).child(message.clone()),
    }
}
