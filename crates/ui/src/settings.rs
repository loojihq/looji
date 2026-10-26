//! The app's preferences, held as a gpui global. Views read them; changes go through [`update`].

use gpui::{App, Global};
use looji_domain::{Preference, Preferences, RepoError, preferences};

use crate::database::AppDatabase;

pub struct AppPreferences(pub Preferences);

impl Global for AppPreferences {}

/// The preferences currently in effect.
pub fn current(cx: &App) -> Preferences {
    cx.global::<AppPreferences>().0.clone()
}

/// Stores a change, then updates memory. A failed write leaves the old value in place, so the
/// screen never shows a choice that wasn't saved.
pub fn update(cx: &mut App, change: Preference) -> Result<(), RepoError> {
    preferences::save(&cx.global::<AppDatabase>().store, &change)?;
    cx.global_mut::<AppPreferences>().0.apply(&change);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_cycle_visits_every_choice_then_returns_to_the_default() {
        let mut current: Option<String> = None;
        let mut seen = Vec::new();
        for _ in 0..=TEMPORARY_MODEL_CHOICES.len() {
            current = next_model(current.as_deref());
            seen.push(current.clone());
        }
        assert_eq!(seen.last(), Some(&None));
        assert_eq!(seen.len(), TEMPORARY_MODEL_CHOICES.len() + 1);
    }

    #[test]
    fn an_unknown_stored_model_cycles_back_to_the_start() {
        assert_eq!(
            next_model(Some("retired-model")),
            Some(TEMPORARY_MODEL_CHOICES[0].to_string())
        );
    }
}
