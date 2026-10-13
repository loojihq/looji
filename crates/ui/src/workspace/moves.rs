//! Moving a card to another column. The card moves on screen at once. The write runs on a
//! background connection, and the board reloads from the database when it lands. A failed write
//! puts the card back in the column it started in, then reloads so the board matches the database.

use gpui::{prelude::*, *};
use looji_domain::{TaskActivity, TaskMove, Timestamp};
use looji_persistence::{MoveError, SqliteStore};

use super::WorkspaceView;
use super::drag::DropResult;
use crate::database::AppDatabase;

impl WorkspaceView {
    /// Starts a move for a card dropped in another column. A drop into the same column is a
    /// reorder, which the ordering work handles, so it's ignored here.
    pub(super) fn commit_move(&mut self, drop: DropResult, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let mv = match TaskMove::new(
            drop.task,
            project.id(),
            drop.from,
            drop.status,
            &project.visible_statuses(),
        ) {
            Ok(mv) => mv,
            Err(err) => {
                eprintln!("could not move task {}: {err}", drop.task.as_uuid());
                self.status = Some("That column can't take this card right now.".to_string());
                return cx.notify();
            }
        };

        if let Some(task) = self.board.iter_mut().find(|task| task.id() == mv.task()) {
            task.set_status(mv.to());
        }
        cx.notify();

        let path = cx.global::<AppDatabase>().database_path.clone();
        let at = Timestamp::now();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = SqliteStore::open(&path)
                        .map_err(|err| MoveError::Storage(err.to_string()))?;
                    store.move_task(&mv, at)
                })
                .await;
            let _ = this.update(cx, |view, cx| view.finish_move(mv, result, cx));
        })
        .detach();
    }

    fn finish_move(
        &mut self,
        mv: TaskMove,
        result: Result<TaskActivity, MoveError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(_) => self.status = None,
            Err(err) => {
                eprintln!("could not move task {}: {err}", mv.task().as_uuid());
                if let Some(task) = self.board.iter_mut().find(|task| task.id() == mv.task()) {
                    task.set_status(mv.from());
                }
                self.status = Some(move_failure_message(&err).to_string());
            }
        }
        // Reload either way, so the board shows what the database holds.
        self.reload_board(cx);
        cx.notify();
    }
}

fn move_failure_message(err: &MoveError) -> &'static str {
    match err {
        MoveError::Conflict => "That task changed elsewhere, so the board was reloaded.",
        MoveError::NotFound => "That task no longer exists.",
        MoveError::Domain(_) | MoveError::Storage(_) => {
            "Could not move the task. It was put back in its column."
        }
    }
}
