//! Showing and hiding the optional columns, Backlog and In review. A hidden column's tasks move to the
//! nearest shown column at once, and come back when the column is shown again.

use super::{WorkspaceView, store};
use looji_domain::{ProjectRepository, Status};

impl WorkspaceView {
    /// Shows or hides an optional column on the active project's board.
    pub(super) fn request_column_visibility(
        &mut self,
        status: Status,
        visible: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_column_visibility(status, visible, cx);
    }

    /// Saves the flag and the task moves in one transaction, then reloads the project and board.
    /// Nothing changes on screen unless the save succeeds.
    fn set_column_visibility(
        &mut self,
        status: Status,
        visible: bool,
        cx: &mut gpui::Context<Self>,
    ) {
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
