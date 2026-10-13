//! Board geometry in logical pixels, as the last paint measured it. Drop targets and placeholder
//! positions are both computed from these, so what the user sees is what gets dropped.

use looji_domain::{Status, TaskId};

/// A point in logical pixels, relative to the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn distance(self, other: Point) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn minus(self, other: Point) -> Point {
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
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x + self.width
            && point.y >= self.origin.y
            && point.y < self.origin.y + self.height
    }
}

/// A column on the board.
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

/// What the last paint measured. Written by the board's paint callbacks, read by the drag.
#[derive(Debug, Default)]
pub struct BoardLayout {
    pub columns: Vec<ColumnSlot>,
    pub cards: Vec<CardSlot>,
}

impl BoardLayout {
    pub fn card(&self, task: TaskId) -> Option<&CardSlot> {
        self.cards.iter().find(|card| card.task == task)
    }

    /// The column under `point`, if any.
    pub fn column_at(&self, point: Point) -> Option<&ColumnSlot> {
        self.columns
            .iter()
            .find(|column| column.bounds.contains(point))
    }

    pub fn set_columns(&mut self, columns: Vec<ColumnSlot>) {
        self.columns = columns;
    }

    /// Replaces the cards of one column. Other columns keep their cards from the same paint.
    pub fn set_cards(&mut self, status: Status, cards: Vec<CardSlot>) {
        self.cards.retain(|card| card.status != status);
        self.cards.extend(cards);
    }
}
