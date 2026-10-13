//! The drag. A state machine turns timestamped input into press, lift, follow, and release, and a
//! session on top of it tracks where the card would land. Nothing here touches gpui, so the
//! timing, thresholds, and drop results are tested directly.
//!
//! A press on a card does one of three things:
//! - released before [`LONG_PRESS`] without moving: a click, which opens the card;
//! - held still for [`LONG_PRESS`]: the card lifts and follows the pointer;
//! - moved fast from the start (a flick), mostly sideways: the card lifts at once.
//!
//! While lifted, every move recomputes the landing and stores it. A release commits the stored
//! landing, not one recomputed from the release position, so the drop is what was on screen.
//! Every other exit is an [`Input::Cancel`].

use std::time::Duration;

use looji_domain::{Status, TaskId};

use super::landing::{Landing, landing_at};
use super::layout::{BoardLayout, Point};

/// How far the pointer may move, in logical pixels, before the press counts as a move.
pub const SLOP: f32 = 6.0;
/// How long a press must stay still to lift the card.
pub const LONG_PRESS: Duration = Duration::from_millis(450);
/// The speed, in logical pixels per second, at which a move counts as a flick.
pub const FLICK_SPEED: f32 = 1200.0;
/// How much recent movement a flick is measured over.
pub const FLICK_WINDOW: Duration = Duration::from_millis(100);

/// Why a drag ended without a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    Escape,
    /// The pointer left the window while the button was held.
    PointerLeftWindow,
    /// The button was released outside the window.
    ReleasedOutside,
    /// The window stopped being the active window, for example on Alt-Tab.
    FocusLost,
    /// A move arrived with no button held, so the release was lost.
    CaptureLost,
    /// The release was inside the window but outside every column.
    NoDropTarget,
}

/// A card that was moved to a new place: the status column and the index among its other cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropResult {
    pub task: TaskId,
    /// The column the card was in when the drag started.
    pub from: Status,
    pub status: Status,
    pub index: usize,
}

/// How a drag ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))] // The fields are read by the debug controls only.
pub enum Exit {
    Dropped(DropResult),
    /// Released where the card already was. Nothing is written.
    Unchanged,
    Cancelled(CancelReason),
}

/// Where the lifted card came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source {
    pub task: TaskId,
    pub status: Status,
    /// The index among the column's cards, counting the card itself.
    pub index: usize,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// `card_origin` is the card's top-left corner, so the grab point can be kept.
    Down {
        card: TaskId,
        pointer: Point,
        card_origin: Point,
        at: Duration,
    },
    Move {
        pointer: Point,
        at: Duration,
    },
    Up {
        pointer: Point,
    },
    /// The long-press timer fired. It only counts if its generation is still current.
    Tick {
        generation: u64,
    },
    Cancel(CancelReason),
}

/// What the state machine decides. [`Drag`] turns these into [`Command`]s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    /// Start the long-press timer, then send [`Input::Tick`] with this generation.
    ArmTimer {
        generation: u64,
        after: Duration,
    },
    OpenCard(TaskId),
    Lift {
        card: TaskId,
        pointer: Point,
        grab: Point,
    },
    Follow {
        pointer: Point,
    },
    Drop {
        card: TaskId,
        pointer: Point,
    },
    Cancel(CancelReason),
}

/// What the workspace should do after an input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    ArmTimer { generation: u64, after: Duration },
    OpenCard(TaskId),
    Lift(TaskId),
    Follow,
    End(Exit),
}

#[derive(Debug, Clone, Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Pressed {
        card: TaskId,
        origin: Point,
        /// Where the pointer sits on the card, so the card keeps its place under the pointer.
        grab: Point,
        /// Recent samples, kept for the flick check.
        recent: Vec<(Point, Duration)>,
    },
    /// Moved without lifting. The release does nothing.
    Ignored,
    Lifted {
        card: TaskId,
        grab: Point,
        pointer: Point,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DragMachine {
    phase: Phase,
    /// Bumped on every press, cancel, and drop, so a timer from an earlier press does nothing.
    generation: u64,
}

impl DragMachine {
    pub fn is_idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    /// The lifted card and the top-left corner of its overlay, which is the pointer minus the
    /// grab point.
    pub fn lifted_at(&self) -> Option<(TaskId, Point)> {
        match self.phase {
            Phase::Lifted {
                card,
                grab,
                pointer,
            } => Some((card, pointer.minus(grab))),
            _ => None,
        }
    }

    pub fn feed(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Down {
                card,
                pointer,
                card_origin,
                at,
            } => {
                if !self.is_idle() {
                    return Vec::new();
                }
                self.generation += 1;
                self.phase = Phase::Pressed {
                    card,
                    origin: pointer,
                    grab: pointer.minus(card_origin),
                    recent: vec![(pointer, at)],
                };
                vec![Effect::ArmTimer {
                    generation: self.generation,
                    after: LONG_PRESS,
                }]
            }
            Input::Move { pointer, at } => self.move_to(pointer, at),
            Input::Up { pointer } => self.release(pointer),
            Input::Tick { generation } => self.long_press(generation),
            Input::Cancel(reason) => {
                if self.is_idle() {
                    return Vec::new();
                }
                self.phase = Phase::Idle;
                self.generation += 1;
                vec![Effect::Cancel(reason)]
            }
        }
    }

    fn move_to(&mut self, pointer: Point, at: Duration) -> Vec<Effect> {
        match &mut self.phase {
            Phase::Pressed {
                card,
                origin,
                grab,
                recent,
            } => {
                recent.push((pointer, at));
                recent.retain(|(_, sampled)| at.saturating_sub(*sampled) <= FLICK_WINDOW);
                if origin.distance(pointer) <= SLOP {
                    return Vec::new();
                }
                let (card, grab) = (*card, *grab);
                if is_flick(recent) {
                    self.phase = Phase::Lifted {
                        card,
                        grab,
                        pointer,
                    };
                    return vec![Effect::Lift {
                        card,
                        pointer,
                        grab,
                    }];
                }
                // Moved, but not a flick: a slow move or a scroll. It won't lift, and the
                // release won't open the card.
                self.phase = Phase::Ignored;
                Vec::new()
            }
            Phase::Lifted {
                pointer: current, ..
            } => {
                *current = pointer;
                vec![Effect::Follow { pointer }]
            }
            Phase::Idle | Phase::Ignored => Vec::new(),
        }
    }

    fn long_press(&mut self, generation: u64) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        let Phase::Pressed {
            card, origin, grab, ..
        } = self.phase
        else {
            return Vec::new();
        };
        self.phase = Phase::Lifted {
            card,
            grab,
            pointer: origin,
        };
        vec![Effect::Lift {
            card,
            pointer: origin,
            grab,
        }]
    }

    fn release(&mut self, pointer: Point) -> Vec<Effect> {
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        if matches!(phase, Phase::Idle) {
            return Vec::new();
        }
        self.generation += 1;
        match phase {
            Phase::Pressed { card, origin, .. } if origin.distance(pointer) <= SLOP => {
                vec![Effect::OpenCard(card)]
            }
            Phase::Lifted { card, .. } => vec![Effect::Drop { card, pointer }],
            _ => Vec::new(),
        }
    }
}

/// A flick is fast movement that starts with the press and runs mostly sideways. Mostly
/// vertical movement is a scroll, so it doesn't lift. Vertical reordering is done after a
/// long press instead.
fn is_flick(recent: &[(Point, Duration)]) -> bool {
    let (Some(&(first, first_at)), Some(&(latest, latest_at))) = (recent.first(), recent.last())
    else {
        return false;
    };
    let seconds = latest_at.saturating_sub(first_at).as_secs_f32();
    if seconds <= 0.0 {
        return false;
    }
    let dx = latest.x - first.x;
    let dy = latest.y - first.y;
    let speed = first.distance(latest) / seconds;
    speed > FLICK_SPEED && dx.abs() >= dy.abs()
}

/// The drag as the workspace sees it: the machine, plus where the card came from and where it
/// would land.
#[derive(Debug, Default)]
pub struct Drag {
    machine: DragMachine,
    source: Option<Source>,
    /// Recomputed on every move while lifted. The release commits this, not a fresh calculation.
    landing: Option<Landing>,
    /// The landing before the most recent change, so the board can animate from it.
    previous: Option<Landing>,
}

impl Drag {
    /// Feeds one input and returns what the workspace should do. `layout` is the board as the
    /// last paint measured it.
    pub fn feed(&mut self, input: Input, layout: &BoardLayout) -> Vec<Command> {
        let mut commands = Vec::new();
        for effect in self.machine.feed(input) {
            match effect {
                Effect::ArmTimer { generation, after } => {
                    commands.push(Command::ArmTimer { generation, after })
                }
                Effect::OpenCard(task) => commands.push(Command::OpenCard(task)),
                Effect::Lift { card, pointer, .. } => {
                    self.source = source_of(card, layout);
                    self.track(pointer, layout);
                    commands.push(Command::Lift(card));
                }
                Effect::Follow { pointer } => {
                    self.track(pointer, layout);
                    commands.push(Command::Follow);
                }
                Effect::Drop { card, .. } => {
                    let exit = self.finish(card);
                    commands.push(Command::End(exit));
                }
                Effect::Cancel(reason) => {
                    self.clear();
                    commands.push(Command::End(Exit::Cancelled(reason)));
                }
            }
        }
        commands
    }

    /// The lifted card and the top-left corner of its overlay.
    pub fn lifted_at(&self) -> Option<(TaskId, Point)> {
        self.machine.lifted_at()
    }

    pub fn source(&self) -> Option<Source> {
        self.source
    }

    pub fn landing(&self) -> Option<Landing> {
        self.landing
    }

    pub fn previous(&self) -> Option<Landing> {
        self.previous
    }

    /// Stores the landing for `pointer`. Only the last change is kept as `previous`.
    fn track(&mut self, pointer: Point, layout: &BoardLayout) {
        let Some(source) = self.source else {
            return;
        };
        let landing = landing_at(layout, source.task, pointer, self.landing);
        if landing != self.landing {
            self.previous = self.landing;
            self.landing = landing;
        }
    }

    /// Commits the stored landing. Dropping on the source's own index is a no-op.
    fn finish(&mut self, card: TaskId) -> Exit {
        let exit = match (self.source, self.landing) {
            (Some(source), Some(landing))
                if landing.status == source.status && landing.index == source.index =>
            {
                Exit::Unchanged
            }
            (Some(source), Some(landing)) => Exit::Dropped(DropResult {
                task: card,
                from: source.status,
                status: landing.status,
                index: landing.index,
            }),
            _ => Exit::Cancelled(CancelReason::NoDropTarget),
        };
        self.clear();
        exit
    }

    fn clear(&mut self) {
        self.source = None;
        self.landing = None;
        self.previous = None;
    }
}

/// The source of a card: its column and its position there. `None` if it isn't on the board.
fn source_of(task: TaskId, layout: &BoardLayout) -> Option<Source> {
    let slot = layout.card(task)?;
    let index = layout
        .cards
        .iter()
        .filter(|card| card.status == slot.status)
        .position(|card| card.task == task)?;
    Some(Source {
        task,
        status: slot.status,
        index,
        height: slot.bounds.height,
    })
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use std::time::Duration;

    use looji_domain::{Status, TaskId};
    use uuid::Uuid;

    use super::{
        CancelReason, Command, Drag, DragMachine, DropResult, Effect, Exit, Input, LONG_PRESS,
    };
    use crate::workspace::layout::{BoardLayout, CardSlot, ColumnSlot, Point, Rect};

    fn task(n: u128) -> TaskId {
        TaskId::from_uuid(Uuid::from_u128(n))
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn point(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// A press on card 1 whose top-left corner is at (100, 100).
    fn press(machine: &mut DragMachine, ms: u64) -> Vec<Effect> {
        machine.feed(Input::Down {
            card: task(1),
            pointer: point(120.0, 110.0),
            card_origin: point(100.0, 100.0),
            at: at(ms),
        })
    }

    fn generation_of(effects: &[Effect]) -> u64 {
        match effects {
            [Effect::ArmTimer { generation, .. }] => *generation,
            other => panic!("expected a timer, got {other:?}"),
        }
    }

    #[test]
    fn a_quick_release_without_moving_opens_the_card() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        let effects = machine.feed(Input::Up {
            pointer: point(120.0, 110.0),
        });
        assert_eq!(effects, [Effect::OpenCard(task(1))]);
        assert!(machine.is_idle());
    }

    #[test]
    fn a_release_within_the_slop_still_opens_the_card() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        let effects = machine.feed(Input::Up {
            pointer: point(124.0, 110.0),
        });
        assert_eq!(effects, [Effect::OpenCard(task(1))]);
    }

    #[test]
    fn holding_still_for_the_long_press_time_lifts_the_card() {
        let mut machine = DragMachine::default();
        let generation = generation_of(&press(&mut machine, 0));
        let effects = machine.feed(Input::Tick { generation });
        assert_eq!(
            effects,
            [Effect::Lift {
                card: task(1),
                pointer: point(120.0, 110.0),
                grab: point(20.0, 10.0),
            }]
        );
        assert_eq!(
            machine.lifted_at(),
            Some((task(1), point(100.0, 100.0))),
            "the overlay keeps the grab point under the pointer"
        );
    }

    #[test]
    fn movement_beyond_the_slop_cancels_the_long_press() {
        let mut machine = DragMachine::default();
        let generation = generation_of(&press(&mut machine, 0));
        // Slow, so it's not a flick.
        machine.feed(Input::Move {
            pointer: point(140.0, 110.0),
            at: at(300),
        });
        assert!(machine.feed(Input::Tick { generation }).is_empty());
        assert!(machine.lifted_at().is_none());
        let effects = machine.feed(Input::Up {
            pointer: point(140.0, 110.0),
        });
        assert!(
            effects.is_empty(),
            "a scroll-like move doesn't open the card"
        );
    }

    #[test]
    fn movement_within_the_slop_keeps_the_long_press() {
        let mut machine = DragMachine::default();
        let generation = generation_of(&press(&mut machine, 0));
        machine.feed(Input::Move {
            pointer: point(124.0, 110.0),
            at: at(100),
        });
        let effects = machine.feed(Input::Tick { generation });
        assert!(matches!(effects.as_slice(), [Effect::Lift { .. }]));
    }

    #[test]
    fn a_fast_sideways_move_from_the_start_lifts_at_once() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        // 60 px in 30 ms is 2000 px/s, and it's all horizontal.
        let effects = machine.feed(Input::Move {
            pointer: point(180.0, 110.0),
            at: at(30),
        });
        assert_eq!(
            effects,
            [Effect::Lift {
                card: task(1),
                pointer: point(180.0, 110.0),
                grab: point(20.0, 10.0),
            }]
        );
    }

    #[test]
    fn a_slow_move_is_not_a_flick() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        let effects = machine.feed(Input::Move {
            pointer: point(180.0, 110.0),
            at: at(500),
        });
        assert!(effects.is_empty());
        assert!(machine.lifted_at().is_none());
    }

    #[test]
    fn a_fast_vertical_move_is_a_scroll_not_a_flick() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        let effects = machine.feed(Input::Move {
            pointer: point(120.0, 170.0),
            at: at(30),
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn a_stale_timer_does_nothing() {
        let mut machine = DragMachine::default();
        let first = generation_of(&press(&mut machine, 0));
        machine.feed(Input::Up {
            pointer: point(120.0, 110.0),
        });
        let second = generation_of(&press(&mut machine, 1000));
        assert_ne!(first, second);

        assert!(machine.feed(Input::Tick { generation: first }).is_empty());
        assert!(machine.lifted_at().is_none());
        assert!(matches!(
            machine.feed(Input::Tick { generation: second }).as_slice(),
            [Effect::Lift { .. }]
        ));
    }

    #[test]
    fn a_timer_from_a_released_press_does_nothing() {
        let mut machine = DragMachine::default();
        let generation = generation_of(&press(&mut machine, 0));
        machine.feed(Input::Up {
            pointer: point(120.0, 110.0),
        });
        assert!(machine.feed(Input::Tick { generation }).is_empty());
        assert!(machine.is_idle());
    }

    #[test]
    fn a_lifted_card_follows_the_pointer_and_a_release_drops_it() {
        let mut machine = DragMachine::default();
        let generation = generation_of(&press(&mut machine, 0));
        machine.feed(Input::Tick { generation });

        let follow = machine.feed(Input::Move {
            pointer: point(300.0, 200.0),
            at: at(600),
        });
        assert_eq!(
            follow,
            [Effect::Follow {
                pointer: point(300.0, 200.0)
            }]
        );

        let drop = machine.feed(Input::Up {
            pointer: point(300.0, 200.0),
        });
        assert_eq!(
            drop,
            [Effect::Drop {
                card: task(1),
                pointer: point(300.0, 200.0),
            }]
        );
        assert!(machine.is_idle());
    }

    #[test]
    fn cancel_from_any_state_returns_to_idle() {
        let reason = CancelReason::Escape;

        let mut pressed = DragMachine::default();
        press(&mut pressed, 0);

        let mut ignored = DragMachine::default();
        press(&mut ignored, 0);
        ignored.feed(Input::Move {
            pointer: point(180.0, 110.0),
            at: at(500),
        });

        let mut lifted = DragMachine::default();
        let generation = generation_of(&press(&mut lifted, 0));
        lifted.feed(Input::Tick { generation });

        for mut machine in [pressed, ignored, lifted] {
            assert_eq!(
                machine.feed(Input::Cancel(reason)),
                [Effect::Cancel(reason)]
            );
            assert!(machine.is_idle());
        }
    }

    #[test]
    fn cancel_while_idle_does_nothing() {
        let mut machine = DragMachine::default();
        assert!(
            machine
                .feed(Input::Cancel(CancelReason::FocusLost))
                .is_empty()
        );
    }

    #[test]
    fn a_second_press_while_busy_is_ignored() {
        let mut machine = DragMachine::default();
        press(&mut machine, 0);
        let effects = machine.feed(Input::Down {
            card: task(2),
            pointer: point(10.0, 10.0),
            card_origin: point(0.0, 0.0),
            at: at(50),
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn long_press_time_is_the_timer_delay() {
        let mut machine = DragMachine::default();
        let effects = press(&mut machine, 0);
        assert!(matches!(
            effects.as_slice(),
            [Effect::ArmTimer { after, .. }] if *after == LONG_PRESS
        ));
    }

    // The board for the drag tests. Todo has A (top 0), D (top 60), and B (top 120), each 50 px
    // tall, and Done is empty. Column Todo spans x 0–100 and Done spans x 120–220.
    fn board() -> BoardLayout {
        board_with(&[(1, 0.0), (2, 60.0), (3, 120.0)])
    }

    /// The same board with a card at each `(task, top)` in Todo.
    fn board_with(cards: &[(u128, f32)]) -> BoardLayout {
        let card = |n: u128, y: f32| CardSlot {
            task: task(n),
            status: Status::Todo,
            bounds: Rect {
                origin: point(0.0, y),
                width: 100.0,
                height: 50.0,
            },
        };
        let column = |status: Status, x: f32| ColumnSlot {
            status,
            bounds: Rect {
                origin: point(x, 0.0),
                width: 100.0,
                height: 500.0,
            },
        };
        BoardLayout {
            columns: vec![column(Status::Todo, 0.0), column(Status::Done, 120.0)],
            cards: cards.iter().map(|&(n, y)| card(n, y)).collect(),
            ..Default::default()
        }
    }

    /// Presses D (task 2) at its origin, holds it until it lifts, and returns the drag.
    fn lifted_d(layout: &BoardLayout) -> Drag {
        let mut drag = Drag::default();
        drag.feed(
            Input::Down {
                card: task(2),
                pointer: point(50.0, 85.0),
                card_origin: point(0.0, 60.0),
                at: at(0),
            },
            layout,
        );
        let commands = drag.feed(Input::Tick { generation: 1 }, layout);
        assert_eq!(commands, [Command::Lift(task(2))]);
        drag
    }

    fn end_of(commands: &[Command]) -> Exit {
        match commands {
            [Command::End(exit)] => *exit,
            other => panic!("expected the drag to end, got {other:?}"),
        }
    }

    #[test]
    fn lifting_keeps_the_placeholder_at_the_source_index() {
        let layout = board();
        let drag = lifted_d(&layout);
        let source = drag.source().expect("the card has a source");
        assert_eq!(source.index, 1);
        assert_eq!(drag.landing().map(|landing| landing.index), Some(1));
    }

    #[test]
    fn the_placeholder_index_is_the_drop_index_at_the_top() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 5.0),
                at: at(100),
            },
            &layout,
        );
        assert_eq!(drag.landing().map(|l| l.index), Some(0));
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 5.0),
            },
            &layout,
        ));
        assert_eq!(
            exit,
            Exit::Dropped(DropResult {
                task: task(2),
                from: Status::Todo,
                status: Status::Todo,
                index: 0
            })
        );
    }

    #[test]
    fn the_placeholder_index_is_the_drop_index_in_the_middle() {
        // D is dragged from index 1 of four. Past B's band and before C's, D lands at index 2.
        let layout = board_with(&[(1, 0.0), (2, 60.0), (3, 120.0), (4, 180.0)]);
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 170.0),
                at: at(100),
            },
            &layout,
        );
        let landing = drag.landing().map(|l| l.index);
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 170.0),
            },
            &layout,
        ));
        assert_eq!(landing, Some(2));
        assert_eq!(
            exit,
            Exit::Dropped(DropResult {
                task: task(2),
                from: Status::Todo,
                status: Status::Todo,
                index: 2
            })
        );
    }

    #[test]
    fn the_middle_of_a_three_card_column_is_the_source_position() {
        // With three cards, D's middle is its own slot, so there's nothing to write.
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 130.0),
                at: at(100),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 130.0),
            },
            &layout,
        ));
        assert_eq!(exit, Exit::Unchanged);
    }

    #[test]
    fn the_placeholder_index_is_the_drop_index_at_the_bottom() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 400.0),
                at: at(100),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 400.0),
            },
            &layout,
        ));
        assert_eq!(
            exit,
            Exit::Dropped(DropResult {
                task: task(2),
                from: Status::Todo,
                status: Status::Todo,
                index: 2
            })
        );
    }

    #[test]
    fn the_placeholder_index_is_the_drop_index_in_an_empty_column() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(150.0, 200.0),
                at: at(100),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(150.0, 200.0),
            },
            &layout,
        ));
        assert_eq!(
            exit,
            Exit::Dropped(DropResult {
                task: task(2),
                from: Status::Todo,
                status: Status::Done,
                index: 0
            })
        );
    }

    #[test]
    fn releasing_over_the_source_position_is_a_no_op() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 400.0),
                at: at(100),
            },
            &layout,
        );
        drag.feed(
            Input::Move {
                pointer: point(50.0, 85.0),
                at: at(200),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 85.0),
            },
            &layout,
        ));
        assert_eq!(exit, Exit::Unchanged);
    }

    #[test]
    fn the_drop_uses_the_last_stored_landing_not_the_release_position() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        // The last move leaves the landing at the top.
        drag.feed(
            Input::Move {
                pointer: point(50.0, 5.0),
                at: at(100),
            },
            &layout,
        );
        // The release arrives at a spot that would land lower. It must be ignored.
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(50.0, 400.0),
            },
            &layout,
        ));
        assert_eq!(
            exit,
            Exit::Dropped(DropResult {
                task: task(2),
                from: Status::Todo,
                status: Status::Todo,
                index: 0
            })
        );
    }

    #[test]
    fn a_release_after_the_drag_has_no_landing_is_cancelled() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(500.0, 200.0),
                at: at(100),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(
            Input::Up {
                pointer: point(500.0, 200.0),
            },
            &layout,
        ));
        assert_eq!(exit, Exit::Cancelled(CancelReason::NoDropTarget));
    }

    #[test]
    fn escape_ends_the_drag_and_clears_the_landing() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 400.0),
                at: at(100),
            },
            &layout,
        );
        let exit = end_of(&drag.feed(Input::Cancel(CancelReason::Escape), &layout));
        assert_eq!(exit, Exit::Cancelled(CancelReason::Escape));
        assert!(drag.landing().is_none());
        assert!(drag.source().is_none());
    }

    #[test]
    fn the_previous_landing_is_kept_for_the_animation() {
        let layout = board();
        let mut drag = lifted_d(&layout);
        drag.feed(
            Input::Move {
                pointer: point(50.0, 5.0),
                at: at(100),
            },
            &layout,
        );
        assert_eq!(drag.previous().map(|l| l.index), Some(1));
        assert_eq!(drag.landing().map(|l| l.index), Some(0));
    }
}
