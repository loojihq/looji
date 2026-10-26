//! Deleting a workspace: the confirmation dialog, and the background delete it starts.

use gpui::{prelude::*, *};
use looji_domain::{WorkspaceId, WorkspaceRepository};
use looji_persistence::{SqliteStore, icons::remove_icon};

use super::{WorkspaceView, state, store};
use crate::theme::Theme;

/// The confirmation dialog's state. `None` on the view means no dialog is open.
#[derive(Debug, Clone)]
pub(super) struct PendingDelete {
    pub id: WorkspaceId,
    pub name: String,
    pub projects: u64,
    pub tasks: u64,
    /// Set once the user confirms. Stops a second confirm from starting another delete.
    pub deleting: bool,
}

impl WorkspaceView {
    /// Opens the confirmation dialog. Only reads counts, so cancelling leaves no trace.
    pub(super) fn begin_delete(
        &mut self,
        id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .list
            .find(id)
            .map(|workspace| workspace.name().to_string())
        else {
            return;
        };
        match store(cx).workspace_summary(id) {
            Ok(summary) => {
                self.cancel_rename(cx);
                self.pending_delete = Some(PendingDelete {
                    id,
                    name,
                    projects: summary.projects,
                    tasks: summary.tasks,
                    deleting: false,
                });
                // The confirmation shows in the workspace screen's delete card, so that screen opens.
                self.open_screen(super::screen::Screen::Workspace, cx);
                self.focus_handle.focus(window, cx);
            }
            Err(err) => self.status = Some(format!("Could not open the delete dialog: {err}")),
        }
        cx.notify();
    }

    /// Closes the dialog without changing anything. Ignored while a delete is running.
    pub(super) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self
            .pending_delete
            .as_ref()
            .is_some_and(|pending| !pending.deleting)
        {
            self.pending_delete = None;
            cx.notify();
        }
    }

    /// Runs the delete on a background thread with its own connection, then reloads the list.
    pub(super) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_delete.as_mut() else {
            return;
        };
        if pending.deleting {
            return;
        }
        pending.deleting = true;
        let id = pending.id;

        // Deleting the selected workspace selects its neighbour. Deleting any other keeps the selection.
        let active = self.list.active_workspace_id();
        let next = if id == active {
            state::neighbour_after(self.list.items(), id)
        } else {
            Some(active)
        };

        let database = cx.global::<crate::database::AppDatabase>();
        let (path, icons_dir) = (database.database_path.clone(), database.icons_dir.clone());
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let store = SqliteStore::open(&path).map_err(|err| err.to_string())?;
                    let deleted = store
                        .delete_workspace(id, next)
                        .map_err(|err| err.to_string())?;
                    // Only after the commit. A missing file is fine.
                    for file in &deleted.icon_files {
                        let _ = remove_icon(&icons_dir, file);
                    }
                    Ok::<WorkspaceId, String>(deleted.active)
                })
                .await;
            let _ = this.update(cx, |view, cx| view.finish_delete(outcome, cx));
        })
        .detach();
    }

    fn finish_delete(&mut self, outcome: Result<WorkspaceId, String>, cx: &mut Context<Self>) {
        self.pending_delete = None;
        match outcome {
            Ok(active) => match WorkspaceRepository::list(store(cx)) {
                Ok(items) => {
                    self.list.reload(items, active);
                    self.reload_projects(cx);
                    self.status = None;
                }
                Err(err) => {
                    self.status = Some(format!(
                        "Deleted, but the list could not be reloaded: {err}"
                    ));
                }
            },
            Err(message) => {
                self.status = Some(format!("Could not delete the workspace: {message}"))
            }
        }
        cx.notify();
    }
}

/// The confirmation, drawn in the workspace screen's delete card in place of its button. Cancel
/// closes it. Enter and Escape also cancel, since Cancel is the default action.
pub(super) fn dialog(
    pending: &PendingDelete,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let counts = format!(
        "{} {}, {} {}",
        pending.projects,
        plural(pending.projects, "project", "projects"),
        pending.tasks,
        plural(pending.tasks, "task", "tasks"),
    );
    let confirm_label = if pending.deleting {
        "Deleting…"
    } else {
        "Delete"
    };

    div()
        .id("delete-panel")
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .w_full()
        .p(theme.spacing.lg)
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.palette.danger)
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(theme.typography.title)
                .child(format!("Delete \"{}\"?", pending.name)),
        )
        .child(div().text_size(theme.typography.body).child(format!(
            "This permanently removes {counts} in this workspace, along with its icon. \
                     This can't be undone."
        )))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(theme.spacing.sm)
                .child(cancel_button(theme, cx))
                .child(confirm_button(confirm_label, pending.deleting, theme, cx)),
        )
}

fn cancel_button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    div()
        .id("delete-cancel")
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .border_1()
        .border_color(theme.palette.border_control)
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child("Cancel")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_delete(cx)))
}

fn confirm_button(
    label: &'static str,
    disabled: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let button = div()
        .id("delete-confirm")
        .px(theme.spacing.md)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .child(label);
    if disabled {
        // Neutral while the delete runs, so the destructive colour isn't shown on a disabled control.
        button
            .bg(theme.palette.surface_selected)
            .text_color(theme.palette.text_disabled)
    } else {
        crate::components::button(
            "delete-confirm",
            label,
            crate::components::Variant::Danger,
            theme,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.confirm_delete(cx)))
    }
}

fn plural(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}
