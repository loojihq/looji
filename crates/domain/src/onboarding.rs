//! The first-run tour: what it says, how it moves, and when it shows. Pure.
//!
//! The tour shows once. Whatever the user does with it, finishing it or skipping it, a value is saved, and any saved
//! value stops it from showing again. A value this version doesn't know counts the same way, so a setting written by a
//! newer version can never bring the tour back.

/// The settings key the tour's outcome is saved under.
pub const TOUR_SETTING: &str = "onboarding.tour";

/// Saved when the user skips the tour.
pub const DISMISSED: &str = "dismissed";
/// Saved when the user reaches the end of the tour.
pub const FINISHED: &str = "finished";

/// Whether the tour should show, given what's saved. Only a user who has never seen it gets it.
pub fn should_show(saved: Option<&str>) -> bool {
    saved.is_none()
}

/// One screen of the tour.
#[derive(Debug, PartialEq, Eq)]
pub struct TourStep {
    pub title: &'static str,
    pub body: &'static str,
}

/// The screens, in order. Each says what to do next, in the words the buttons use.
pub const STEPS: &[TourStep] = &[
    TourStep {
        title: "Welcome to looji",
        body: "A workspace holds your projects. You start with one called Personal. Add more with + New workspace in the sidebar.",
    },
    TourStep {
        title: "Projects and the board",
        body: "Create a project from the sidebar. Its board has five columns: Backlog, To do, In progress, In review, and Done.",
    },
    TourStep {
        title: "Add and move tasks",
        body: "Use + New task to add a card. It starts in To do. Drag it to another column, or Tab to it, press Space, use the arrow keys, and press Enter to drop it.",
    },
    TourStep {
        title: "Plan and chat with AI",
        body: "Plan turns a goal into a spec, stories, and a backlog. Open a task and choose Chat to ask about it. Both need an Anthropic API key.",
    },
    TourStep {
        title: "Connect your code",
        body: "Repository links a project to a local git folder, including a folder inside WSL. Chat can then use that code when it answers.",
    },
    TourStep {
        title: "Reports and updates",
        body: "Export PDF writes the board as a report. When a new version is out, a notice appears in the corner, and Update now installs it.",
    },
];

/// Where the user is in the tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tour {
    index: usize,
}

/// What a press of Next did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Moved to the next screen.
    Moved,
    /// Was on the last screen, so the tour is over.
    Finished,
}

impl Tour {
    pub fn start() -> Self {
        Self { index: 0 }
    }

    pub fn step(&self) -> &'static TourStep {
        &STEPS[self.index]
    }

    /// The screen number counting from one, for "2 of 6".
    pub fn position(&self) -> usize {
        self.index + 1
    }

    pub fn count(&self) -> usize {
        STEPS.len()
    }

    pub fn is_first(&self) -> bool {
        self.index == 0
    }

    pub fn is_last(&self) -> bool {
        self.index + 1 == STEPS.len()
    }

    pub fn advance(&mut self) -> Advance {
        if self.is_last() {
            return Advance::Finished;
        }
        self.index += 1;
        Advance::Moved
    }

    pub fn retreat(&mut self) {
        self.index = self.index.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use crate::onboarding::{Advance, DISMISSED, FINISHED, STEPS, Tour, should_show};

    #[test]
    fn a_new_user_sees_the_tour() {
        assert!(should_show(None));
    }

    #[test]
    fn a_user_who_skipped_or_finished_never_sees_it_again() {
        assert!(!should_show(Some(DISMISSED)));
        assert!(!should_show(Some(FINISHED)));
    }

    #[test]
    fn any_saved_value_keeps_the_tour_away_even_one_this_version_does_not_know() {
        assert!(!should_show(Some("")));
        assert!(!should_show(Some("snoozed-by-a-newer-version")));
    }

    #[test]
    fn the_tour_walks_forward_and_back_and_ends_after_the_last_screen() {
        let mut tour = Tour::start();
        assert!(tour.is_first());
        assert_eq!(tour.position(), 1);

        tour.retreat();
        assert_eq!(tour.position(), 1, "back from the first screen stays put");

        let mut moves = 0;
        while tour.advance() == Advance::Moved {
            moves += 1;
        }
        assert_eq!(moves, STEPS.len() - 1);
        assert!(tour.is_last());
        assert_eq!(
            tour.advance(),
            Advance::Finished,
            "next on the last screen ends the tour"
        );

        tour.retreat();
        assert_eq!(tour.position(), STEPS.len() - 1);
    }

    #[test]
    fn every_screen_has_words_and_stays_short() {
        assert!(STEPS.len() >= 4 && STEPS.len() <= 8, "a short tour");
        for step in STEPS {
            assert!(!step.title.trim().is_empty());
            assert!(
                step.body.len() > 20 && step.body.len() < 220,
                "{}",
                step.title
            );
        }
    }
}
