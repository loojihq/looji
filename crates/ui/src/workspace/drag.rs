//! The drag state machine. Pure logic: timestamped input goes in, effects come out. Nothing here
//! touches gpui, so the timing and thresholds are tested directly.
//!
//! A press on a card does one of three things:
//! - released before [`LONG_PRESS`] without moving: a click, which opens the card;
//! - held still for [`LONG_PRESS`]: the card lifts and follows the pointer;
//! - moved fast from the start (a flick), mostly sideways: the card lifts at once.
//!
//! A release while lifted drops the card. Every other exit is an [`Input::Cancel`].

use std::time::Duration;

use looji_domain::{Status, TaskId};

/// How far the pointer may move, in logical pixels, before the press counts as a move.
pub const SLOP: f32 = 6.0;
/// How long a press must stay still to lift the card.
pub const LONG_PRESS: Duration = Duration::from_millis(450);
/// The speed, in logical pixels per second, at which a move counts as a flick.
pub const FLICK_SPEED: f32 = 1200.0;
/// How much recent movement a flick is measured over.
pub const FLICK_WINDOW: Duration = Duration::from_millis(100);

/// A point in logical pixels, relative to the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    fn distance(self, other: Point) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    fn minus(self, other: Point) -> Point {
        Point {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x + self.width
            && point.y >= self.origin.y
            && point.y < self.origin.y + self.height
    }

    fn mid_y(&self) -> f32 {
        self.origin.y + self.height / 2.0
    }
}

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

/// A column on the board, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnSlot {
    pub status: Status,
    pub bounds: Rect,
}

/// A card on the board, in the order it appears in its column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardSlot {
    pub task: TaskId,
    pub status: Status,
    pub bounds: Rect,
}

/// Where a drag landed: the status column and the index among that column's other cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropResult {
    pub task: TaskId,
    pub status: Status,
    pub index: usize,
}

/// The column under the pointer, and the index for the card there: the number of other cards
/// whose midpoint is above the pointer. `None` if the pointer is outside every column.
pub fn resolve_drop(
    dragged: TaskId,
    pointer: Point,
    columns: &[ColumnSlot],
    cards: &[CardSlot],
) -> Option<DropResult> {
    let column = columns
        .iter()
        .find(|column| column.bounds.contains(pointer))?;
    let index = cards
        .iter()
        .filter(|card| {
            card.status == column.status && card.task != dragged && card.bounds.mid_y() < pointer.y
        })
        .count();
    Some(DropResult {
        task: dragged,
        status: column.status,
        index,
    })
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use std::time::Duration;

    use looji_domain::{Status, TaskId};
    use uuid::Uuid;

    use super::{
        CancelReason, CardSlot, ColumnSlot, DragMachine, Effect, Input, LONG_PRESS, Point, Rect,
        resolve_drop,
    };

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

    fn column(status: Status, x: f32) -> ColumnSlot {
        ColumnSlot {
            status,
            bounds: Rect {
                origin: point(x, 0.0),
                width: 100.0,
                height: 500.0,
            },
        }
    }

    fn card(n: u128, status: Status, y: f32) -> CardSlot {
        CardSlot {
            task: task(n),
            status,
            bounds: Rect {
                origin: point(0.0, y),
                width: 100.0,
                height: 50.0,
            },
        }
    }

    #[test]
    fn a_drop_outside_every_column_has_no_target() {
        let columns = [column(Status::Todo, 0.0)];
        assert_eq!(
            resolve_drop(task(9), point(500.0, 10.0), &columns, &[]),
            None
        );
    }

    #[test]
    fn a_drop_in_an_empty_column_is_index_zero() {
        let columns = [column(Status::Todo, 0.0), column(Status::Done, 120.0)];
        let result = resolve_drop(task(9), point(150.0, 200.0), &columns, &[]).unwrap();
        assert_eq!(result.status, Status::Done);
        assert_eq!(result.index, 0);
        assert_eq!(result.task, task(9));
    }

    #[test]
    fn the_index_counts_cards_whose_midpoint_is_above_the_pointer() {
        let columns = [column(Status::Todo, 0.0)];
        let cards = [
            card(1, Status::Todo, 0.0),   // midpoint 25
            card(2, Status::Todo, 60.0),  // midpoint 85
            card(3, Status::Todo, 120.0), // midpoint 145
        ];
        let result = resolve_drop(task(9), point(50.0, 100.0), &columns, &cards).unwrap();
        assert_eq!(result.index, 2);
        assert_eq!(result.status, Status::Todo);
    }

    #[test]
    fn the_dragged_card_is_not_counted_and_other_columns_are_ignored() {
        let columns = [column(Status::Todo, 0.0)];
        let cards = [
            card(9, Status::Todo, 0.0),   // the card being dragged
            card(2, Status::Done, 60.0),  // a card in another column
            card(3, Status::Todo, 120.0), // below the pointer
        ];
        let result = resolve_drop(task(9), point(50.0, 100.0), &columns, &cards).unwrap();
        assert_eq!(result.index, 0);
    }
}
