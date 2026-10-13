//! Showing and hiding the optional In review column, and the confirmation for hiding one with tasks.

use gpui::{prelude::*, *};
use looji_domain::{ProjectRepository, Status, resolve_visible_status, visible_statuses};

use super::{WorkspaceView, store};
use crate::theme::Theme;

/// A hide that is waiting for confirmation, because the column has tasks in it.
#[derive(Debug, Clone)]
pub(super) struct PendingColumn {
    pub status: Status,
    pub count: usize,
    /// Where the tasks will go.
    pub target: Status,
}

impl WorkspaceView {
    /// Hides or shows a column. Hiding a column that has tasks opens a confirmation first.
    pub(super) fn request_column_visibility(
        &mut self,
        status: Status,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        if !visible {
            let count = self
                .board
                .iter()
                .filter(|task| task.status() == status)
                .count();
            if count > 0 {
                let target = resolve_visible_status(status, &visible_statuses(&[status]));
                self.pending_column = Some(PendingColumn {
                    status,
                    count,
                    target,
                });
                cx.notify();
                return;
            }
        }
        self.set_column_visibility(status, visible, cx);
    }

    pub(super) fn confirm_column_hide(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_column.take() {
            self.set_column_visibility(pending.status, false, cx);
        }
    }

    pub(super) fn cancel_column_hide(&mut self, cx: &mut Context<Self>) {
        if self.pending_column.take().is_some() {
            cx.notify();
        }
    }

    /// Saves the flag and the task moves in one transaction, then reloads the project and board.
    /// Nothing changes on screen unless the save succeeds.
    fn set_column_visibility(&mut self, status: Status, visible: bool, cx: &mut Context<Self>) {
        let Some(project_id) = self.projects.active_id() else {
            return;
        };
        match store(cx).set_status_visible(project_id, status, visible) {
            Ok(_) => {
                let workspace = self.list.active_workspace_id();
                if let Ok(Some(project)) = ProjectRepository::find(store(cx), workspace, project_id)
                {
                    self.projects.replace(project);
                }
                self.status = None;
                self.reload_board(cx);
            }
            Err(err) => self.status = Some(format!("Could not change the column: {err}")),
        }
        cx.notify();
    }
}

/// The confirmation for hiding a column that has tasks.
pub(super) fn dialog(
    pending: &PendingColumn,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let noun = if pending.count == 1 { "task" } else { "tasks" };
    div()
        .id("column-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.palette.scrim)
        .occlude()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_column_hide(cx)))
        .child(
            div()
                .id("column-panel")
                .flex()
                .flex_col()
                .gap(theme.spacing.md)
                .w(px(380.))
                .p(theme.spacing.lg)
                .rounded(theme.radius.lg)
                .bg(theme.palette.surface_raised)
                .shadow(theme.elevation.overlay.clone())
                .border_1()
                .border_color(theme.palette.border)
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(theme.typography.title)
                        .child(format!("Hide {}?", status_name(pending.status))),
                )
                .child(
                    div()
                        .text_size(theme.typography.body)
                        .child(format!(
                            "{} {} in {} will move to {}. They return to {} when the column is shown again, unless you have moved them.",
                            pending.count,
                            noun,
                            status_name(pending.status),
                            status_name(pending.target),
                            status_name(pending.status),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(theme.spacing.sm)
                        .child(
                            div()
                                .id("column-cancel")
                                .px(theme.spacing.md)
                                .py(theme.spacing.xs)
                                .rounded(theme.radius.md)
                                .border_1()
                                .border_color(theme.palette.border_control)
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.palette.surface_hover))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cancel_column_hide(cx)
                                })),
                        )
                        .child(
                            div()
                                .id("column-hide")
                                .px(theme.spacing.md)
                                .py(theme.spacing.xs)
                                .rounded(theme.radius.md)
                                .bg(theme.palette.accent)
                                .text_color(theme.palette.on_accent)
                                .cursor_pointer()
                                .child("Hide column")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.confirm_column_hide(cx)
                                })),
                        ),
                ),
        )
        .into_any_element()
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Backlog => "Backlog",
        Status::Todo => "To do",
        Status::InProgress => "In progress",
        Status::InReview => "In review",
        Status::Done => "Done",
    }
}
