//! Which statuses a board shows, and where tasks go when their status is hidden.
//!
//! Pure functions only. The persistence layer applies the results inside a transaction.

use crate::{error::DomainError, status::Status};

/// The statuses left visible when `hidden` are hidden, in board order.
pub fn visible_statuses(hidden: &[Status]) -> Vec<Status> {
    Status::ALL
        .into_iter()
        .filter(|status| !hidden.contains(status))
        .collect()
}

/// Fails for the four default statuses, which are always shown and can't be hidden.
pub fn check_hideable(status: Status) -> Result<(), DomainError> {
    if status.is_default() {
        Err(DomainError::StatusNotHideable {
            status: status.as_str().to_string(),
        })
    } else {
        Ok(())
    }
}

/// The status a task moves to when its own status is hidden: the nearest status in `visible`,
/// measured along `Status::ALL`.
///
/// If two visible statuses are equally near, the less advanced one wins. A hidden In review
/// therefore falls back to In progress, never to Done. `visible` must not be empty; the four
/// defaults are always visible, so a real board never has an empty set.
pub fn resolve_visible_status(status: Status, visible: &[Status]) -> Status {
    if visible.contains(&status) {
        return status;
    }
    let position = |candidate: Status| {
        Status::ALL
            .iter()
            .position(|known| *known == candidate)
            .expect("every status is in Status::ALL")
    };
    let target = position(status);
    visible
        .iter()
        .copied()
        .min_by_key(|candidate| {
            let index = position(*candidate);
            (index.abs_diff(target), index)
        })
        .expect("the visible set always contains the default statuses")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visible_status_resolves_to_itself() {
        let visible = visible_statuses(&[]);
        assert_eq!(
            resolve_visible_status(Status::InReview, &visible),
            Status::InReview
        );
    }

    #[test]
    fn a_hidden_status_falls_to_the_nearest_visible_one() {
        // In review is hidden. In progress and Done are both one step away, and In progress is
        // less advanced, so it wins the tie.
        let visible = visible_statuses(&[Status::InReview]);
        assert_eq!(
            resolve_visible_status(Status::InReview, &visible),
            Status::InProgress
        );
    }

    #[test]
    fn the_tie_breaks_toward_the_less_advanced_status() {
        // Only Done is visible above In review, and Todo is visible below. Todo is two steps
        // away and Done is one, so Done wins on distance even though it's more advanced.
        let visible = [Status::Backlog, Status::Todo, Status::Done];
        assert_eq!(
            resolve_visible_status(Status::InReview, &visible),
            Status::Done
        );

        // Equal distance: In progress (one step below) and Done (one step above). In progress wins.
        let visible = [Status::Backlog, Status::InProgress, Status::Done];
        assert_eq!(
            resolve_visible_status(Status::InReview, &visible),
            Status::InProgress
        );
    }

    #[test]
    fn a_hidden_nearest_status_is_skipped_for_the_next_visible_one() {
        // In progress is hidden from this set. Todo is one step back and Done is two, so Todo wins.
        let visible = [Status::Backlog, Status::Todo, Status::Done];
        assert_eq!(
            resolve_visible_status(Status::InProgress, &visible),
            Status::Todo
        );
    }

    #[test]
    fn the_four_defaults_cannot_be_hidden() {
        for status in Status::DEFAULTS {
            assert_eq!(
                check_hideable(status),
                Err(DomainError::StatusNotHideable {
                    status: status.as_str().to_string()
                })
            );
        }
        assert_eq!(check_hideable(Status::InReview), Ok(()));
    }

    #[test]
    fn hiding_leaves_the_defaults_visible() {
        let visible = visible_statuses(&[Status::InReview]);
        for status in Status::DEFAULTS {
            assert!(visible.contains(&status));
        }
        assert!(!visible.contains(&Status::InReview));
    }
}
