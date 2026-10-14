//! Moving a card with the keyboard, as a pure state machine. Space picks up the focused card, the
//! arrows move it, Enter drops it and Escape cancels. Nothing is written until Enter, and each step
//! says what it did, so the same text can be shown to the user and read aloud.
//!
//! A place is a status and an index among that column's other cards. The held card is left out of
//! its column, the same as the mouse drag, so a keyboard move and a mouse drop agree on the index.

use gpui::{prelude::*, *};
use looji_domain::{Status, TaskId};

use super::WorkspaceView;
use super::board::status_label;
use super::drag::{DropResult, source_of};
use super::landing::Landing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Space,
    Enter,
    Escape,
    Left,
    Right,
    Up,
    Down,
}

impl Key {
    /// The key for a GPUI keystroke name, or `None` for keys the board doesn't use.
    pub fn from_name(name: &str) -> Option<Key> {
        Some(match name {
            "space" => Key::Space,
            "enter" => Key::Enter,
            "escape" => Key::Escape,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            _ => return None,
        })
    }
}

/// A status column and its cards, top to bottom. Only shown columns are listed, so the arrows
/// skip hidden optional columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub status: Status,
    pub tasks: Vec<TaskId>,
}

/// A position on the board: a status and an index among that column's other cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub status: Status,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hold {
    #[default]
    Idle,
    Holding {
        task: TaskId,
        origin: Place,
        target: Place,
    },
}

/// What a step asks the board to do beyond changing the hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Enter on a place that differs from the origin: persist the move.
    Commit {
        task: TaskId,
        from: Place,
        to: Place,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub hold: Hold,
    /// Whether the key was used. An unused key goes on to the rest of the app.
    pub consumed: bool,
    /// What happened, in words, for the user and for screen readers.
    pub announce: Option<String>,
    pub outcome: Outcome,
}

impl Step {
    fn unchanged(hold: Hold) -> Step {
        Step {
            hold,
            consumed: false,
            announce: None,
            outcome: Outcome::None,
        }
    }
}

/// Advances the keyboard move by one key. `focused` is the card that has focus, if any.
pub fn step(columns: &[Column], hold: Hold, focused: Option<TaskId>, key: Key) -> Step {
    match hold {
        Hold::Idle => idle(columns, focused, key),
        Hold::Holding {
            task,
            origin,
            target,
        } => holding(columns, task, origin, target, key),
    }
}

fn idle(columns: &[Column], focused: Option<TaskId>, key: Key) -> Step {
    let hold = Hold::Idle;
    let (Key::Space, Some(task)) = (key, focused) else {
        return Step::unchanged(hold);
    };
    let Some(origin) = place_of(columns, task) else {
        return Step::unchanged(hold);
    };
    let hold = Hold::Holding {
        task,
        origin,
        target: origin,
    };
    Step {
        hold,
        consumed: true,
        announce: Some(format!(
            "Picked up. {}, position {} of {}. Arrow keys move it, Enter drops it, Escape cancels.",
            status_label(origin.status),
            origin.index + 1,
            others(columns, task, origin.status) + 1,
        )),
        outcome: Outcome::None,
    }
}

fn holding(columns: &[Column], task: TaskId, origin: Place, target: Place, key: Key) -> Step {
    let holding = |target| Hold::Holding {
        task,
        origin,
        target,
    };
    match key {
        Key::Left | Key::Right => {
            let Some(position) = columns
                .iter()
                .position(|column| column.status == target.status)
            else {
                return Step::unchanged(holding(target));
            };
            let next = if key == Key::Left {
                position.checked_sub(1)
            } else {
                Some(position + 1).filter(|next| *next < columns.len())
            };
            let Some(next) = next else {
                return held_at_edge(task, origin, target, edge_word(key));
            };
            let status = columns[next].status;
            // The same row if the new column is long enough, otherwise its last place.
            let index = target.index.min(others(columns, task, status));
            let moved = Place { status, index };
            moved_to(columns, task, moved, holding(moved))
        }
        Key::Up | Key::Down => {
            let last = others(columns, task, target.status);
            let index = if key == Key::Up {
                target.index.checked_sub(1)
            } else {
                Some(target.index + 1).filter(|index| *index <= last)
            };
            let Some(index) = index else {
                return held_at_edge(task, origin, target, edge_word(key));
            };
            let moved = Place {
                status: target.status,
                index,
            };
            moved_to(columns, task, moved, holding(moved))
        }
        Key::Enter => {
            let announce = if target == origin {
                "Dropped where it was. Nothing changed.".to_string()
            } else {
                format!(
                    "Dropped. {}, position {} of {}.",
                    status_label(target.status),
                    target.index + 1,
                    others(columns, task, target.status) + 1,
                )
            };
            Step {
                hold: Hold::Idle,
                consumed: true,
                announce: Some(announce),
                outcome: if target == origin {
                    Outcome::None
                } else {
                    Outcome::Commit {
                        task,
                        from: origin,
                        to: target,
                    }
                },
            }
        }
        Key::Escape => Step {
            hold: Hold::Idle,
            consumed: true,
            announce: Some(format!(
                "Move cancelled. {}, position {} of {}.",
                status_label(origin.status),
                origin.index + 1,
                others(columns, task, origin.status) + 1,
            )),
            outcome: Outcome::None,
        },
        Key::Space => Step {
            hold: holding(target),
            consumed: true,
            announce: None,
            outcome: Outcome::None,
        },
    }
}

/// A move that stopped at the edge of the board. Nothing changes, and the user is told why.
fn held_at_edge(task: TaskId, origin: Place, target: Place, word: &str) -> Step {
    Step {
        hold: Hold::Holding {
            task,
            origin,
            target,
        },
        consumed: true,
        announce: Some(format!("Already at the {word}.")),
        outcome: Outcome::None,
    }
}

fn moved_to(columns: &[Column], task: TaskId, moved: Place, hold: Hold) -> Step {
    Step {
        hold,
        consumed: true,
        announce: Some(format!(
            "{}, position {} of {}.",
            status_label(moved.status),
            moved.index + 1,
            others(columns, task, moved.status) + 1,
        )),
        outcome: Outcome::None,
    }
}

fn edge_word(key: Key) -> &'static str {
    match key {
        Key::Left => "first column",
        Key::Right => "last column",
        Key::Up => "top",
        _ => "bottom",
    }
}

/// The place a task is at, or `None` if it isn't on the board.
pub fn place_of(columns: &[Column], task: TaskId) -> Option<Place> {
    columns.iter().find_map(|column| {
        let index = column.tasks.iter().position(|id| *id == task)?;
        Some(Place {
            status: column.status,
            index,
        })
    })
}

/// How many cards a column has, not counting the held card.
fn others(columns: &[Column], task: TaskId, status: Status) -> usize {
    columns
        .iter()
        .find(|column| column.status == status)
        .map_or(0, |column| {
            column.tasks.iter().filter(|id| **id != task).count()
        })
}

impl WorkspaceView {
    /// The board as the keyboard sees it: each shown column with its cards, in display order.
    fn keyboard_columns(&self) -> Vec<Column> {
        let Some(project) = self.projects.active() else {
            return Vec::new();
        };
        project
            .visible_statuses()
            .into_iter()
            .map(|status| Column {
                status,
                tasks: self
                    .board
                    .iter()
                    .filter(|task| task.status() == status)
                    .map(|task| task.id())
                    .collect(),
            })
            .collect()
    }

    /// A key pressed on a focused card. The key is used only when the move uses it, so the rest
    /// of the app still gets keys like Escape otherwise.
    pub(super) fn card_key(
        &mut self,
        task: TaskId,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While a panel field is being typed in, the keys belong to the panel, so the card lets them through.
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.draft.is_some())
        {
            return;
        }
        // The dialog owns the keys while it's open, even when a card behind it has focus.
        if self.task_dialog.is_some() {
            self.task_dialog_key(event, cx);
            return cx.stop_propagation();
        }
        if self.team_dialog.is_some() {
            self.team_dialog_key(event, cx);
            return cx.stop_propagation();
        }
        let Some(key) = Key::from_name(&event.keystroke.key) else {
            return;
        };
        if self.keyboard_key(Some(task), key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Carries one key through the keyboard move and does what the step asks. Returns whether the
    /// key was used.
    pub(super) fn keyboard_key(
        &mut self,
        focused: Option<TaskId>,
        key: Key,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The held card is looked up on the board as it is on screen, so refresh first.
        self.refresh_layout();
        let columns = self.keyboard_columns();
        let before = self.drag.keyboard;
        let step = step(&columns, before, focused, key);
        self.drag.keyboard = step.hold;
        if let Some(text) = step.announce {
            self.drag.keyboard_notice = Some(text);
        }

        match step.hold {
            Hold::Holding { task, target, .. } => {
                if before == Hold::Idle {
                    self.watch_keyboard_focus(window, cx);
                    // The held card leaves its slot while the placeholder shows, so keys go to the
                    // workspace until the card is dropped or cancelled.
                    self.focus_handle.focus(window);
                }
                // The hold shows what a mouse drag shows: the source dimmed, and the landing as
                // the placeholder.
                // The source is taken at pick-up. Afterwards the held card is out of its column's
                // cards, so it can't be looked up again.
                let source = self
                    .drag
                    .session
                    .source()
                    .filter(|source| source.task == task)
                    .or_else(|| {
                        let layout = self.drag.layout.borrow();
                        source_of(task, &layout)
                    });
                if let Some(source) = source {
                    self.drag.session.show_hold(
                        source,
                        Landing {
                            status: target.status,
                            index: target.index,
                        },
                    );
                }
                self.drag
                    .scroll_handle(target.status)
                    .scroll_to_item(target.index);
            }
            Hold::Idle => {
                if let Hold::Holding { task: held, .. } = before {
                    self.drag.session.clear_hold();
                    self.drag.keyboard_activation = None;
                    // Dropped or cancelled, focus stays on the card that was moved.
                    self.drag.focus_handle(held, cx).focus(window);
                }
            }
        }

        if let Outcome::Commit { task, from, to } = step.outcome {
            self.commit_move(
                DropResult {
                    task,
                    from: from.status,
                    status: to.status,
                    index: to.index,
                },
                cx,
            );
        }
        cx.notify();
        step.consumed
    }

    /// Cancels a keyboard move from outside it: a mouse press, or the window losing focus.
    pub(super) fn cancel_keyboard(&mut self, cx: &mut Context<Self>) {
        if self.drag.keyboard == Hold::Idle {
            return;
        }
        self.drag.keyboard = Hold::Idle;
        self.drag.session.clear_hold();
        self.drag.keyboard_activation = None;
        self.drag.keyboard_notice = Some("Move cancelled.".to_string());
        cx.notify();
    }

    /// A held card that is no longer on the board cancels the move, rather than writing a stale
    /// target.
    pub(super) fn cancel_keyboard_if_gone(&mut self, cx: &mut Context<Self>) {
        if let Hold::Holding { task, .. } = self.drag.keyboard
            && !self.board.iter().any(|other| other.id() == task)
        {
            self.drag.keyboard = Hold::Idle;
            self.drag.session.clear_hold();
            self.drag.keyboard_activation = None;
            self.drag.keyboard_notice =
                Some("That card was removed, so the move was cancelled.".to_string());
            cx.notify();
        }
    }

    /// Losing focus while a card is held cancels the hold, so nothing is left half-moved.
    fn watch_keyboard_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drag.keyboard_activation =
            Some(cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.cancel_keyboard(cx);
                }
            }));
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{Status, TaskId};
    use uuid::Uuid;

    use super::{Column, Hold, Key, Outcome, Place, place_of, step};

    fn task(n: u128) -> TaskId {
        TaskId::from_uuid(Uuid::from_u128(n))
    }

    /// Todo has 1, 2, 3; In progress has 4; Done is empty.
    fn board() -> Vec<Column> {
        vec![
            Column {
                status: Status::Todo,
                tasks: vec![task(1), task(2), task(3)],
            },
            Column {
                status: Status::InProgress,
                tasks: vec![task(4)],
            },
            Column {
                status: Status::Done,
                tasks: vec![],
            },
        ]
    }

    fn pick(columns: &[Column], focus: TaskId) -> Hold {
        step(columns, Hold::Idle, Some(focus), Key::Space).hold
    }

    fn holding_at(hold: Hold) -> (TaskId, Place, Place) {
        match hold {
            Hold::Holding {
                task,
                origin,
                target,
            } => (task, origin, target),
            Hold::Idle => panic!("expected a hold"),
        }
    }

    #[test]
    fn space_picks_up_the_focused_card_where_it_is() {
        let columns = board();
        let hold = pick(&columns, task(2));
        let (held, origin, target) = holding_at(hold);
        assert_eq!(held, task(2));
        assert_eq!(
            origin,
            Place {
                status: Status::Todo,
                index: 1
            }
        );
        assert_eq!(target, origin);
    }

    #[test]
    fn space_without_focus_does_nothing() {
        let step = step(&board(), Hold::Idle, None, Key::Space);
        assert!(!step.consumed);
        assert_eq!(step.hold, Hold::Idle);
    }

    #[test]
    fn up_and_down_move_one_place_within_the_column() {
        let columns = board();
        let held = pick(&columns, task(2)); // at index 1 of Todo
        let up = step(&columns, held, None, Key::Up);
        assert_eq!(
            holding_at(up.hold).2,
            Place {
                status: Status::Todo,
                index: 0
            }
        );
        let down = step(&columns, held, None, Key::Down);
        assert_eq!(
            holding_at(down.hold).2,
            Place {
                status: Status::Todo,
                index: 2
            }
        );
    }

    #[test]
    fn up_at_the_top_and_down_at_the_bottom_stop() {
        let columns = board();
        let top = pick(&columns, task(1));
        let stop = step(&columns, top, None, Key::Up);
        assert_eq!(holding_at(stop.hold).2.index, 0, "no wrap to the bottom");
        assert_eq!(stop.announce.as_deref(), Some("Already at the top."));

        // The card at index 2 of Todo; the others are 1 and 2, so the last place is index 2.
        let bottom = pick(&columns, task(3));
        let stop = step(&columns, bottom, None, Key::Down);
        assert_eq!(holding_at(stop.hold).2.index, 2);
        assert_eq!(stop.announce.as_deref(), Some("Already at the bottom."));
    }

    #[test]
    fn right_moves_to_the_next_column_and_keeps_the_row_when_it_fits() {
        let columns = board();
        let held = pick(&columns, task(2)); // Todo index 1
        let right = step(&columns, held, None, Key::Right);
        let (_, _, target) = holding_at(right.hold);
        assert_eq!(
            target,
            Place {
                status: Status::InProgress,
                index: 1
            }
        );
        assert_eq!(
            right.announce.as_deref(),
            Some("In progress, position 2 of 2.")
        );
    }

    #[test]
    fn right_into_an_empty_column_lands_at_index_zero() {
        let columns = board();
        let held = pick(&columns, task(4));
        let to_done = step(&columns, held, None, Key::Right);
        let (_, _, target) = holding_at(to_done.hold);
        assert_eq!(
            target,
            Place {
                status: Status::Done,
                index: 0
            }
        );
    }

    #[test]
    fn left_from_the_first_column_and_right_from_the_last_stop() {
        let columns = board();
        let held = pick(&columns, task(1));
        let stop = step(&columns, held, None, Key::Left);
        assert_eq!(holding_at(stop.hold).2.status, Status::Todo, "no wrap");
        assert_eq!(
            stop.announce.as_deref(),
            Some("Already at the first column.")
        );

        let held = pick(&columns, task(4));
        let to_done = step(&columns, held, None, Key::Right);
        let stop = step(&columns, to_done.hold, None, Key::Right);
        assert_eq!(holding_at(stop.hold).2.status, Status::Done, "no wrap");
        assert_eq!(
            stop.announce.as_deref(),
            Some("Already at the last column.")
        );
    }

    #[test]
    fn hidden_columns_are_skipped_because_they_are_not_listed() {
        // In review is hidden, so Todo's right neighbour is In progress, and then Done.
        let columns = board();
        let held = pick(&columns, task(1));
        let first = step(&columns, held, None, Key::Right);
        let second = step(&columns, first.hold, None, Key::Right);
        assert_eq!(holding_at(second.hold).2.status, Status::Done);
    }

    #[test]
    fn enter_commits_a_real_move_and_goes_idle() {
        let columns = board();
        let held = pick(&columns, task(2));
        let moved = step(&columns, held, None, Key::Right);
        let dropped = step(&columns, moved.hold, None, Key::Enter);
        assert_eq!(dropped.hold, Hold::Idle);
        assert_eq!(
            dropped.outcome,
            Outcome::Commit {
                task: task(2),
                from: Place {
                    status: Status::Todo,
                    index: 1
                },
                to: Place {
                    status: Status::InProgress,
                    index: 1
                },
            }
        );
    }

    #[test]
    fn enter_where_the_card_already_is_writes_nothing() {
        let columns = board();
        let held = pick(&columns, task(2));
        let dropped = step(&columns, held, None, Key::Enter);
        assert_eq!(dropped.hold, Hold::Idle);
        assert_eq!(dropped.outcome, Outcome::None);
    }

    #[test]
    fn escape_cancels_and_returns_to_the_origin_without_a_commit() {
        let columns = board();
        let held = pick(&columns, task(2));
        let moved = step(&columns, held, None, Key::Down);
        let cancelled = step(&columns, moved.hold, None, Key::Escape);
        assert_eq!(cancelled.hold, Hold::Idle);
        assert_eq!(cancelled.outcome, Outcome::None);
        assert_eq!(
            cancelled.announce.as_deref(),
            Some("Move cancelled. To do, position 2 of 3.")
        );
    }

    #[test]
    fn keys_outside_a_hold_are_not_consumed() {
        let columns = board();
        for key in [
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Enter,
            Key::Escape,
        ] {
            let step = step(&columns, Hold::Idle, Some(task(1)), key);
            assert!(
                !step.consumed,
                "{key:?} should go on to the rest of the app"
            );
        }
    }

    #[test]
    fn a_card_not_on_the_board_cannot_be_picked_up() {
        assert_eq!(place_of(&board(), task(99)), None);
    }
}

#[cfg(test)]
mod persistence_tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{
        Position, Project, ProjectId, ProjectRepository, Status, Task, TaskId, TaskMove,
        TaskRepository, Timestamp, Workspace, WorkspaceRepository,
    };
    use looji_persistence::SqliteStore;

    use super::{Column, Hold, Key, Outcome, step};

    fn project(store: &SqliteStore) -> Project {
        let workspace = Workspace::new("Work").unwrap();
        WorkspaceRepository::save(store, &workspace).unwrap();
        let project = Project::new(workspace.id(), "Website").unwrap();
        ProjectRepository::save(store, &project).unwrap();
        project
    }

    fn create(store: &SqliteStore, project: ProjectId, status: Status, key: &str) -> Task {
        let task = Task::new(project, "Write copy", status, Position::new(key).unwrap()).unwrap();
        TaskRepository::save(store, &task).unwrap();
        task
    }

    fn find(store: &SqliteStore, task: TaskId) -> Task {
        TaskRepository::find(store, task).unwrap().unwrap()
    }

    /// The board as the keyboard sees it, from the database.
    fn columns(store: &SqliteStore, project: ProjectId) -> Vec<Column> {
        let tasks = TaskRepository::list_for_project(store, project).unwrap();
        Status::DEFAULTS
            .into_iter()
            .map(|status| Column {
                status,
                tasks: tasks
                    .iter()
                    .filter(|task| task.status() == status)
                    .map(|task| task.id())
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn space_right_up_enter_writes_only_on_enter_and_lands_where_shown() {
        let store = SqliteStore::open_in_memory().unwrap();
        let website = project(&store);
        let project_id = website.id();
        let first = create(&store, project_id, Status::Todo, "a0");
        let moved = create(&store, project_id, Status::Todo, "a1");
        let other = create(&store, project_id, Status::InProgress, "a0");

        let picked = step(
            &columns(&store, project_id),
            Hold::Idle,
            Some(moved.id()),
            Key::Space,
        );
        let right = step(&columns(&store, project_id), picked.hold, None, Key::Right);
        let up = step(&columns(&store, project_id), right.hold, None, Key::Up);
        // Nothing has been written yet: the card is still where it started.
        assert_eq!(find(&store, moved.id()).status(), Status::Todo);
        assert_eq!(find(&store, moved.id()).position().as_str(), "a1");

        let dropped = step(&columns(&store, project_id), up.hold, None, Key::Enter);
        let Outcome::Commit { task, from, to } = dropped.outcome else {
            panic!("Enter should commit the move, got {:?}", dropped.outcome);
        };
        let mv = TaskMove::new(
            task,
            project_id,
            from.status,
            to.status,
            to.index,
            &Status::DEFAULTS,
        )
        .unwrap();
        store.move_task(&mv, Timestamp::now()).unwrap();

        // Column and position match what the keys showed: In progress, first place.
        assert_eq!(find(&store, moved.id()).status(), Status::InProgress);
        let order: Vec<TaskId> = columns(&store, project_id)
            .into_iter()
            .find(|column| column.status == Status::InProgress)
            .unwrap()
            .tasks;
        assert_eq!(order, [moved.id(), other.id()]);
        assert_eq!(find(&store, first.id()).status(), Status::Todo);
    }
}
