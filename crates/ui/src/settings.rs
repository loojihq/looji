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
