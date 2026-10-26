//! The AI provider and model. The app sets one of each in Preferences. A project can set its own in its settings,
//! and where it doesn't, the app's choice applies. The model's default is the last resort. Every AI call asks here,
//! so the choice is made once and used everywhere.

use looji_ai::Provider;
use looji_domain::{ProjectId, SettingsRepository};

use super::{WorkspaceView, store};
use crate::settings;

/// The app's provider setting. Claude Code when nothing has been chosen.
const PROVIDER_KEY: &str = "ai_provider";
/// A project's own provider and model, stored under the project's ID.
const PROJECT_PROVIDER: &str = "ai_provider";
const PROJECT_MODEL: &str = "ai_model";

fn project_key(project: ProjectId, name: &str) -> String {
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

/// The next choice in a cycle: `None` is "use the app's", then each entry in turn, then back to `None`.
pub(super) fn next_choice<T: PartialEq + Copy>(current: Option<T>, all: &[T]) -> Option<T> {
    let choices: Vec<Option<T>> = std::iter::once(None)
        .chain(all.iter().map(|item| Some(*item)))
        .collect();
    let index = choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or(0);
    choices[(index + 1) % choices.len()]
}

impl WorkspaceView {
    /// Saves the app's provider and moves it to the next one.
    pub(super) fn cycle_app_provider(&mut self, cx: &mut gpui::Context<Self>) {
        let next =
            next_choice(Some(app_provider(cx)), &Provider::ALL).unwrap_or(Provider::ClaudeCode);
        let result = store(cx).set_setting(PROVIDER_KEY, next.stored());
        self.save_ai(result, cx);
    }

    /// Saves the app's model and moves it to the next one. `None` is the default.
    pub(super) fn cycle_app_model(&mut self, cx: &mut gpui::Context<Self>) {
        let current = settings::current(cx).selected_model;
        let next = next_choice(current.as_deref(), looji_ai::models::MODELS);
        self.save_preference(
            looji_domain::Preference::SelectedModel(next.map(str::to_string)),
            cx,
        );
    }

    /// Moves a project's provider through: the app's choice, then each provider.
    pub(super) fn cycle_project_provider(
        &mut self,
        project: ProjectId,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = next_choice(project_provider_choice(cx, project), &Provider::ALL);
        let key = project_key(project, PROJECT_PROVIDER);
        let result = match next {
            Some(provider) => store(cx).set_setting(&key, provider.stored()),
            None => store(cx).remove_setting(&key),
        };
        self.save_ai(result, cx);
    }

    /// Moves a project's model through: the app's choice, then each model.
    pub(super) fn cycle_project_model(&mut self, project: ProjectId, cx: &mut gpui::Context<Self>) {
        let current = project_model_choice(cx, project);
        let next = next_choice(current.as_deref(), looji_ai::models::MODELS);
        let key = project_key(project, PROJECT_MODEL);
        let result = match next {
            Some(model) => store(cx).set_setting(&key, model),
            None => store(cx).remove_setting(&key),
        };
        self.save_ai(result, cx);
    }

    /// Saves one AI setting to the app or to a project, and says so if the save fails.
    fn save_preference(&mut self, change: looji_domain::Preference, cx: &mut gpui::Context<Self>) {
        if let Err(err) = settings::update(cx, change) {
            self.status = Some(format!("Could not save the setting: {err}"));
        }
        cx.notify();
    }

    fn save_ai(&mut self, result: looji_domain::RepoResult<()>, cx: &mut gpui::Context<Self>) {
        match result {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("Could not save the AI settings: {err}")),
        }
        cx.notify();
    }
}
