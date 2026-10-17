//! Where a dragged card would land, and how a column's items are stacked around the placeholder.
//! Pure logic. The board renders from these functions and the drop reads the same landing, so
//! the placeholder and the saved position always agree.

use looji_domain::{Status, TaskId};

use super::layout::{BoardLayout, CardSlot, Point};

/// A card's middle band, as fractions of its height. Inside the band the decision is kept from
/// the previous move, so the placeholder doesn't flicker when the pointer rests on a boundary.
const LOWER: f32 = 0.25;
const UPPER: f32 = 0.75;

/// A drop target: a column, and the index among its cards with the dragged card left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landing {
    pub status: Status,
    pub index: usize,
}

/// The index a card dropped at `pointer_y` lands at in a column. `cards` are the column's cards
/// top to bottom, and `dragged` is left out of them, since its own slot is still in the list.
///
/// A card counts as above the pointer when the pointer is past its lower band. A card with the
/// pointer above its upper band doesn't count. Inside the band, `previous` decides: a card that
/// counted last time keeps counting, so a resting pointer doesn't make the placeholder jump.
pub fn landing_index(
    cards: &[CardSlot],
    dragged: TaskId,
    pointer_y: f32,
    previous: Option<usize>,
) -> usize {
    let mut index = 0;
    for card in cards.iter().filter(|card| card.task != dragged) {
        let top = card.bounds.origin.y;
        let height = card.bounds.height;
        let above = if pointer_y < top + height * LOWER {
            false
        } else if pointer_y > top + height * UPPER {
            true
        } else {
            previous.is_some_and(|previous| index < previous)
        };
        if !above {
            break;
        }
        index += 1;
    }
    index
}

/// The landing for a pointer position, or `None` if it's outside every column. `previous` only
/// matters within the same column.
pub fn landing_at(
    layout: &BoardLayout,
    dragged: TaskId,
    pointer: Point,
    previous: Option<Landing>,
) -> Option<Landing> {
    let column = layout.column_at(pointer)?;
    let cards: Vec<CardSlot> = layout
        .cards
        .iter()
        .filter(|card| card.status == column.status)
        .copied()
        .collect();
    let previous = previous
        .filter(|previous| previous.status == column.status)
        .map(|previous| previous.index);
    Some(Landing {
        status: column.status,
        index: landing_index(&cards, dragged, pointer.y, previous),
    })
}

/// One item in a column, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Task(TaskId),
    Placeholder,
}

/// The items of a column in display order. `tasks` are in column order. `hidden` is the card being
/// dragged out of this column: while the placeholder is here, its slot is taken by the placeholder.
/// `placeholder` is the landing index among the remaining tasks.
pub fn column_sequence(
    tasks: &[TaskId],
    hidden: Option<TaskId>,
    placeholder: Option<usize>,
) -> Vec<Slot> {
    let remaining: Vec<TaskId> = tasks
        .iter()
        .copied()
        .filter(|task| Some(*task) != hidden)
        .collect();
    let at = placeholder.map(|index| index.min(remaining.len()));
    let mut slots = Vec::with_capacity(remaining.len() + 1);
    for (index, task) in remaining.iter().enumerate() {
        if at == Some(index) {
            slots.push(Slot::Placeholder);
        }
        slots.push(Slot::Task(*task));
    }
    if at == Some(remaining.len()) {
        slots.push(Slot::Placeholder);
    }
    slots
}

/// The top of each item when they're stacked with `gap` between them, starting at zero.
pub fn stack_tops(heights: &[f32], gap: f32) -> Vec<f32> {
    let mut top = 0.0;
    heights
        .iter()
        .map(|height| {
            let current = top;
            top += height + gap;
            current
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use looji_domain::{Status, TaskId};
    use uuid::Uuid;

    use super::{Landing, Slot, column_sequence, landing_at, landing_index, stack_tops};
    use crate::workspace::layout::{BoardLayout, CardSlot, ColumnSlot, Point, Rect};

    fn task(n: u128) -> TaskId {
        TaskId::from_uuid(Uuid::from_u128(n))
    }

    /// A 50 px card whose top is at `y`, in column Todo.
    fn card(n: u128, y: f32) -> CardSlot {
        CardSlot {
            task: task(n),
            status: Status::Todo,
            bounds: Rect {
                origin: Point { x: 0.0, y },
                width: 100.0,
                height: 50.0,
            },
        }
    }

    const DRAGGED: u128 = 99;

    // Cards A (top 0), B (top 60), C (top 120), each 50 px tall. Bands: A 12.5–37.5,
    // B 72.5–97.5, C 132.5–157.5.
    fn three_cards() -> Vec<CardSlot> {
        vec![card(1, 0.0), card(2, 60.0), card(3, 120.0)]
    }

    #[test]
    fn top_puts_the_landing_before_the_first_card() {
        let cards = three_cards();
        assert_eq!(landing_index(&cards, task(DRAGGED), 5.0, None), 0);
    }

    #[test]
    fn middle_puts_the_landing_between_two_cards() {
        let cards = three_cards();
        // Past A's band, before B's band.
        assert_eq!(landing_index(&cards, task(DRAGGED), 65.0, None), 1);
    }

    #[test]
    fn bottom_puts_the_landing_after_the_last_card() {
        let cards = three_cards();
        assert_eq!(landing_index(&cards, task(DRAGGED), 200.0, None), 3);
    }

    #[test]
    fn an_empty_column_lands_at_zero() {
        assert_eq!(landing_index(&[], task(DRAGGED), 200.0, None), 0);
    }

    #[test]
    fn the_dragged_card_is_not_counted() {
        // Dragging B: the list still has B's slot, but only A and C count.
        let cards = three_cards();
        assert_eq!(landing_index(&cards, task(2), 65.0, None), 1);
        assert_eq!(landing_index(&cards, task(2), 200.0, None), 2);
    }

    #[test]
    fn dropping_over_the_source_position_gives_the_source_index() {
        // B is dragged from index 1 and the pointer rests on its own slot.
        let cards = three_cards();
        assert_eq!(landing_index(&cards, task(2), 70.0, None), 1);
    }

    #[test]
    fn the_lower_band_edge_is_inside_the_dead_zone() {
        let cards = three_cards();
        // Exactly 25% of A's height: no previous decision, so A doesn't count.
        assert_eq!(landing_index(&cards, task(DRAGGED), 12.5, None), 0);
        // With A counted last time, the same point keeps A counted.
        assert_eq!(landing_index(&cards, task(DRAGGED), 12.5, Some(1)), 1);
    }

    #[test]
    fn the_upper_band_edge_is_inside_the_dead_zone() {
        let cards = three_cards();
        // Exactly 75% of A's height: A stays counted only if it was counted before.
        assert_eq!(landing_index(&cards, task(DRAGGED), 37.5, Some(0)), 0);
        assert_eq!(landing_index(&cards, task(DRAGGED), 37.5, Some(1)), 1);
    }

    #[test]
    fn past_the_band_the_previous_decision_does_not_matter() {
        let cards = three_cards();
        assert_eq!(landing_index(&cards, task(DRAGGED), 38.0, Some(0)), 1);
        assert_eq!(landing_index(&cards, task(DRAGGED), 12.0, Some(3)), 0);
    }

    #[test]
    fn a_resting_pointer_inside_the_band_does_not_flicker() {
        let cards = three_cards();
        // Wobbling inside A's band, from either side, keeps the same index.
        for y in [14.0, 20.0, 26.0, 35.0] {
            assert_eq!(landing_index(&cards, task(DRAGGED), y, Some(1)), 1);
            assert_eq!(landing_index(&cards, task(DRAGGED), y, Some(0)), 0);
        }
    }

    #[test]
    fn landing_at_uses_the_column_under_the_pointer() {
        let layout = BoardLayout {
            columns: vec![
                ColumnSlot {
                    status: Status::Todo,
                    bounds: Rect {
                        origin: Point { x: 0.0, y: 0.0 },
                        width: 100.0,
                        height: 500.0,
                    },
                },
                ColumnSlot {
                    status: Status::Done,
                    bounds: Rect {
                        origin: Point { x: 120.0, y: 0.0 },
                        width: 100.0,
                        height: 500.0,
                    },
                },
            ],
            cards: three_cards(),
            ..Default::default()
        };

        let todo = landing_at(&layout, task(DRAGGED), Point { x: 50.0, y: 200.0 }, None);
        assert_eq!(
            todo,
            Some(Landing {
                status: Status::Todo,
                index: 3
            })
        );

        let done = landing_at(&layout, task(DRAGGED), Point { x: 150.0, y: 200.0 }, None);
        assert_eq!(
            done,
            Some(Landing {
                status: Status::Done,
                index: 0
            })
        );

        assert_eq!(
            landing_at(&layout, task(DRAGGED), Point { x: 500.0, y: 200.0 }, None),
            None
        );
    }

    #[test]
    fn a_previous_landing_in_another_column_does_not_hold_the_index() {
        let layout = BoardLayout {
            columns: vec![ColumnSlot {
                status: Status::Todo,
                bounds: Rect {
                    origin: Point { x: 0.0, y: 0.0 },
                    width: 100.0,
                    height: 500.0,
                },
            }],
            cards: three_cards(),
            ..Default::default()
        };
        let previous = Some(Landing {
            status: Status::Done,
            index: 2,
        });
        // The pointer is in the dead zone of A, but the previous landing is in Done, so A's
        // decision comes from the band alone.
        let landing = landing_at(&layout, task(DRAGGED), Point { x: 50.0, y: 25.0 }, previous);
        assert_eq!(landing.map(|landing| landing.index), Some(0));
    }

    #[test]
    fn the_placeholder_goes_at_the_top_middle_and_bottom() {
        let tasks = [task(1), task(2), task(3)];
        assert_eq!(
            column_sequence(&tasks, None, Some(0)),
            [
                Slot::Placeholder,
                Slot::Task(task(1)),
                Slot::Task(task(2)),
                Slot::Task(task(3))
            ]
        );
        assert_eq!(
            column_sequence(&tasks, None, Some(1)),
            [
                Slot::Task(task(1)),
                Slot::Placeholder,
                Slot::Task(task(2)),
                Slot::Task(task(3))
            ]
        );
        assert_eq!(
            column_sequence(&tasks, None, Some(3)),
            [
                Slot::Task(task(1)),
                Slot::Task(task(2)),
                Slot::Task(task(3)),
                Slot::Placeholder
            ]
        );
    }

    #[test]
    fn an_empty_column_shows_only_the_placeholder() {
        assert_eq!(column_sequence(&[], None, Some(0)), [Slot::Placeholder]);
    }

    #[test]
    fn an_index_past_the_end_is_clamped() {
        let tasks = [task(1)];
        assert_eq!(
            column_sequence(&tasks, None, Some(9)),
            [Slot::Task(task(1)), Slot::Placeholder]
        );
    }

    #[test]
    fn the_dragged_card_leaves_its_slot_while_the_placeholder_is_in_the_column() {
        // A, D, B with D dragged from index 1. The placeholder takes D's place.
        let tasks = [task(1), task(2), task(3)];
        assert_eq!(
            column_sequence(&tasks, Some(task(2)), Some(1)),
            [Slot::Task(task(1)), Slot::Placeholder, Slot::Task(task(3))]
        );
    }

    #[test]
    fn stacked_items_have_the_gap_between_them() {
        assert_eq!(stack_tops(&[10.0, 20.0, 5.0], 4.0), [0.0, 14.0, 38.0]);
    }

    #[test]
    fn the_source_position_stacks_exactly_like_no_drag() {
        // Heights of A, D, B with the placeholder the height of D, at D's own index. Nothing moves.
        let heights_before = [30.0, 50.0, 40.0];
        let tasks = [task(1), task(2), task(3)];
        let sequence = column_sequence(&tasks, Some(task(2)), Some(1));
        let heights_during = [30.0, 50.0, 40.0];
        assert_eq!(sequence.len(), heights_during.len());
        assert_eq!(
            stack_tops(&heights_during, 4.0),
            stack_tops(&heights_before, 4.0)
        );
    }
}
