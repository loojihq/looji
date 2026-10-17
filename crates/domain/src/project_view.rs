//! Which view a project was last showing, and where each view was scrolled. Each project keeps its own.
//!
//! A project that has never saved this state opens on the board. The scroll positions are stored by a
//! key per scrollable place (the list, the roadmap, the workload, and each board column). A key that no
//! longer makes sense is ignored, so a value saved by another version can't break the app.

use std::collections::BTreeMap;

/// The views a project can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ProjectView {
    #[default]
    Board,
    List,
    Roadmap,
    Workload,
}

impl ProjectView {
    pub const ALL: [ProjectView; 4] = [
        ProjectView::Board,
        ProjectView::List,
        ProjectView::Roadmap,
        ProjectView::Workload,
    ];

    /// The stored name.
    pub fn name(self) -> &'static str {
        match self {
            ProjectView::Board => "board",
            ProjectView::List => "list",
            ProjectView::Roadmap => "roadmap",
            ProjectView::Workload => "workload",
        }
    }

    /// The view with this stored name. `None` for a name this version doesn't know, which the caller
    /// treats as the default.
    pub fn parse(name: &str) -> Option<Self> {
        ProjectView::ALL
            .into_iter()
            .find(|view| view.name() == name)
    }
}

/// How far a scrollable place is scrolled, as distances from its top left, in pixels. Both are zero or
/// more.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollOffset {
    pub x: f32,
    pub y: f32,
}

/// The key for a scrollable place. The list, roadmap, and workload each have one. The board has one per
/// status column.
pub mod scroll_key {
    pub const LIST: &str = "list";
    pub const ROADMAP: &str = "roadmap";
    pub const WORKLOAD: &str = "workload";

    /// The key for a board column, by its stored status name.
    pub fn board_column(status: &str) -> String {
        format!("board:{status}")
    }
}

/// A project's view state: the view it shows, and the saved scroll position of each place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectViewState {
    pub view: ProjectView,
    pub scroll: BTreeMap<String, ScrollOffset>,
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::project_view::{ProjectView, ProjectViewState, scroll_key};

    #[test]
    fn a_project_with_no_saved_state_shows_the_board() {
        assert_eq!(ProjectView::default(), ProjectView::Board);
        assert_eq!(ProjectViewState::default().view, ProjectView::Board);
        assert!(ProjectViewState::default().scroll.is_empty());
    }

    #[test]
    fn every_view_name_round_trips_and_an_unknown_one_is_refused() {
        for view in ProjectView::ALL {
            assert_eq!(ProjectView::parse(view.name()), Some(view));
        }
        assert_eq!(ProjectView::parse("kanban"), None);
    }

    #[test]
    fn board_columns_have_their_own_keys() {
        assert_eq!(scroll_key::board_column("todo"), "board:todo");
        assert_ne!(
            scroll_key::board_column("todo"),
            scroll_key::board_column("done")
        );
        assert_ne!(scroll_key::LIST, scroll_key::ROADMAP);
    }
}
