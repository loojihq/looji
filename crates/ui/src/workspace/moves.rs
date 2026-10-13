//! Moving a card to another place on the board, in its column or another one. The card moves on
//! screen at once. The write runs on a background connection, and the board reloads from the
//! database when it lands. A failed write puts the card back where it started, then reloads so the
//! board matches the database.

use gpui::{prelude::*, *};
use looji_domain::{TaskActivity, TaskMove, Timestamp};
use looji_persistence::{MoveError, SqliteStore};

use super::WorkspaceView;
use super::drag::DropResult;
use crate::database::AppDatabase;

impl WorkspaceView {
    /// Starts the move for a card dropped at a new place. The write works out the card's key
    /// from the column as the database holds it.
    pub(super) fn commit_move(&mut self, drop: DropResult, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let mv = match TaskMove::new(
            drop.task,
            project.id(),
            drop.from,
            drop.status,
            drop.index,
            &project.visible_statuses(),
        ) {
            Ok(mv) => mv,
            Err(err) => {
                eprintln!("could not move task {}: {err}", drop.task.as_uuid());
                self.status = Some("That column can't take this card right now.".to_string());
                return cx.notify();
            }
        };

        self.place_locally(&mv);
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

    /// Shows the card in its new place straight away. The board's order is the database's order,
    /// so the reload after the write puts it exactly where the database has it.
    fn place_locally(&mut self, mv: &TaskMove) {
        let Some(from) = self.board.iter().position(|task| task.id() == mv.task()) else {
            return;
        };
        let mut task = self.board.remove(from);
        task.set_status(mv.to());
        // The card goes in front of the column's task at its index, or after the column's last
        // task when it lands at the end.
        let column: Vec<usize> = self
            .board
            .iter()
            .enumerate()
            .filter(|(_, other)| other.status() == mv.to())
            .map(|(index, _)| index)
            .collect();
        let at = match column.get(mv.index()) {
            Some(&index) => index,
            None => column.last().map_or(self.board.len(), |&index| index + 1),
        };
        self.board.insert(at, task);
    }

    fn finish_move(
        &mut self,
        mv: TaskMove,
        result: Result<Option<TaskActivity>, MoveError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(_) => self.status = None,
            Err(err) => {
                eprintln!("could not move task {}: {err}", mv.task().as_uuid());
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
            "Could not move the task. It was put back in its place."
        }
    }
}
