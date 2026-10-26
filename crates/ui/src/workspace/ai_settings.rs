//! The AI provider and model. The app sets one of each in Preferences. A project can set its own in its settings,
//! and where it doesn't, the app's choice applies. The model's default is the last resort. Every AI call asks here,
//! so the choice is made once and used everywhere.

use looji_ai::Provider;
use looji_domain::{ProjectId, SettingsRepository};

use super::store;
use crate::settings;

/// The app's provider setting. Claude Code when nothing has been chosen.
pub(super) const PROVIDER_KEY: &str = "ai_provider";
/// A project's own provider and model, stored under the project's ID.
pub(super) const PROJECT_PROVIDER: &str = "ai_provider";
pub(super) const PROJECT_MODEL: &str = "ai_model";

/// The settings-store key for one of a project's own settings.
pub(super) fn project_key(project: ProjectId, name: &str) -> String {
    format!("project.{}.{name}", project.as_uuid())
}

fn project_setting(cx: &gpui::App, project: ProjectId, name: &str) -> Option<String> {
    store(cx)
        .get_setting(&project_key(project, name))
        .ok()
        .flatten()
}

/// The app's provider.
pub(super) fn app_provider(cx: &gpui::App) -> Provider {
    store(cx)
        .get_setting(PROVIDER_KEY)
        .ok()
        .flatten()
        .as_deref()
        .and_then(Provider::from_stored)
        .unwrap_or(Provider::ClaudeCode)
}

/// The provider a project's AI work uses: its own choice, then the app's.
pub(super) fn provider_for(cx: &gpui::App, project: Option<ProjectId>) -> Provider {
    project
        .and_then(|project| project_setting(cx, project, PROJECT_PROVIDER))
        .as_deref()
        .and_then(Provider::from_stored)
        .unwrap_or_else(|| app_provider(cx))
}

/// The model a project's AI work uses: its own choice, then the app's, then the default.
pub(super) fn model_for(cx: &gpui::App, project: Option<ProjectId>) -> String {
    project
        .and_then(|project| project_setting(cx, project, PROJECT_MODEL))
        .or_else(|| settings::current(cx).selected_model)
        .unwrap_or_else(|| looji_ai::models::DEFAULT_MODEL.to_string())
}

/// The project's own provider, if it has one.
pub(super) fn project_provider_choice(cx: &gpui::App, project: ProjectId) -> Option<Provider> {
    project_setting(cx, project, PROJECT_PROVIDER)
        .as_deref()
        .and_then(Provider::from_stored)
}

/// The project's own model, if it has one.
pub(super) fn project_model_choice(cx: &gpui::App, project: ProjectId) -> Option<String> {
    project_setting(cx, project, PROJECT_MODEL)
}
