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

/// Model IDs the picker cycles through. TEMPORARY: the model picker task replaces this with the
/// provider's live list, since the list changes over time.
pub const TEMPORARY_MODEL_CHOICES: &[&str] =
    &["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"];

/// The next model after `current` in the temporary list. `None` is the default, and it comes
/// after the last model and before the first.
pub fn next_model(current: Option<&str>) -> Option<String> {
    let choices: Vec<Option<&str>> = std::iter::once(None)
        .chain(TEMPORARY_MODEL_CHOICES.iter().map(|id| Some(*id)))
        .collect();
    let index = choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or(0);
    choices[(index + 1) % choices.len()].map(str::to_string)
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
