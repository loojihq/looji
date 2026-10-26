//! The dropdowns for the AI and time settings. Preferences has them for the app, and project settings has them for a
//! project. Each one shows its current choice, and a click opens a menu of the options. A project's menu starts with
//! Default, which clears the project's own choice so the app's applies again.

use gpui::{prelude::*, *};
use looji_ai::Provider;
use looji_domain::{Clock, Preference, ProjectId, RepoResult, SettingsRepository, UtcOffset};

use super::{WorkspaceView, ai_settings, store, time_settings};
use crate::{components, settings, theme::Theme};

/// Which setting a dropdown changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Provider,
    Model,
    Clock,
    Offset,
}

/// Whether a dropdown changes the app's setting or one project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    App,
    Project(ProjectId),
}

/// The width of every dropdown in Preferences and project settings.
const SELECT_WIDTH: f32 = 224.;

/// The dropdown that is open, if one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpenMenu {
    pub field: Field,
    pub scope: Scope,
}

/// One value a dropdown can choose. `Default` clears a project's choice, or means the model's default for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Default,
    Provider(Provider),
    Model(String),
    Clock(Clock),
    Offset(UtcOffset),
}

/// The value the setting has when it's left on Default, shown inside the Default row.
fn inherited(field: Field, scope: Scope, cx: &gpui::App) -> String {
    match (field, scope) {
        (Field::Provider, _) => ai_settings::app_provider(cx).label().to_string(),
        (Field::Model, Scope::App) => looji_ai::models::DEFAULT_MODEL.to_string(),
        (Field::Model, Scope::Project(_)) => settings::current(cx)
            .selected_model
            .unwrap_or_else(|| looji_ai::models::DEFAULT_MODEL.to_string()),
        (Field::Clock, _) => time_settings::app_clock(cx).label().to_string(),
        (Field::Offset, _) => time_settings::app_offset(cx).to_string(),
    }
}

/// The choice the setting has now.
fn current(field: Field, scope: Scope, cx: &gpui::App) -> Choice {
    match (field, scope) {
        (Field::Provider, Scope::App) => Choice::Provider(ai_settings::app_provider(cx)),
        (Field::Provider, Scope::Project(project)) => {
            ai_settings::project_provider_choice(cx, project)
                .map_or(Choice::Default, Choice::Provider)
        }
        (Field::Model, Scope::App) => settings::current(cx)
            .selected_model
            .map_or(Choice::Default, Choice::Model),
        (Field::Model, Scope::Project(project)) => {
            ai_settings::project_model_choice(cx, project).map_or(Choice::Default, Choice::Model)
        }
        (Field::Clock, Scope::App) => Choice::Clock(time_settings::app_clock(cx)),
        (Field::Clock, Scope::Project(project)) => {
            time_settings::project_clock_choice(cx, project).map_or(Choice::Default, Choice::Clock)
        }
        (Field::Offset, Scope::App) => Choice::Offset(time_settings::app_offset(cx)),
        (Field::Offset, Scope::Project(project)) => {
            time_settings::project_offset_choice(cx, project)
                .map_or(Choice::Default, Choice::Offset)
        }
    }
}

/// The text a choice shows.
fn label(choice: &Choice, field: Field, scope: Scope, cx: &gpui::App) -> String {
    match choice {
        Choice::Default => format!("Default ({})", inherited(field, scope, cx)),
        Choice::Provider(provider) => provider.label().to_string(),
        Choice::Model(model) => model.clone(),
        Choice::Clock(clock) => clock.label().to_string(),
        Choice::Offset(offset) => offset.to_string(),
    }
}

/// Every choice the menu lists, in order. A project's menu and the app's model menu start with Default.
fn options(field: Field, scope: Scope) -> Vec<Choice> {
    let mut choices = Vec::new();
    if matches!(scope, Scope::Project(_)) || field == Field::Model {
        // Default is also the app's model choice: no model chosen means the newest one.
        choices.push(Choice::Default);
    }
    match field {
        Field::Provider => choices.extend(Provider::ALL.into_iter().map(Choice::Provider)),
        Field::Model => choices.extend(
            looji_ai::models::MODELS
                .iter()
                .map(|model| Choice::Model((*model).to_string())),
        ),
        Field::Clock => choices.extend(Clock::ALL.into_iter().map(Choice::Clock)),
        Field::Offset => choices.extend(
            time_settings::offset_choices()
                .into_iter()
                .map(Choice::Offset),
        ),
    }
    choices
}

/// The name a project's own setting is stored under, after its `project.{id}.` prefix.
fn project_key_name(field: Field) -> &'static str {
    match field {
        Field::Provider => ai_settings::PROJECT_PROVIDER,
        Field::Model => ai_settings::PROJECT_MODEL,
        Field::Clock => time_settings::PROJECT_CLOCK,
        Field::Offset => time_settings::PROJECT_OFFSET,
    }
}

/// Saves a choice to the app or to one project. Default on a project removes its own setting.
fn save(scope: Scope, field: Field, choice: &Choice, cx: &mut gpui::App) -> RepoResult<()> {
    match scope {
        Scope::App => match (field, choice) {
            (Field::Provider, Choice::Provider(provider)) => {
                store(cx).set_setting(ai_settings::PROVIDER_KEY, provider.stored())
            }
            (Field::Model, Choice::Default) => {
                settings::update(cx, Preference::SelectedModel(None))
            }
            (Field::Model, Choice::Model(model)) => {
                settings::update(cx, Preference::SelectedModel(Some(model.clone())))
            }
            (Field::Clock, Choice::Clock(clock)) => settings::update(cx, Preference::Clock(*clock)),
            (Field::Offset, Choice::Offset(offset)) => {
                settings::update(cx, Preference::UtcOffset(*offset))
            }
            _ => Ok(()),
        },
        Scope::Project(project) => {
            let key = ai_settings::project_key(project, project_key_name(field));
            let value = match choice {
                Choice::Default => None,
                Choice::Provider(provider) => Some(provider.stored().to_string()),
                Choice::Model(model) => Some(model.clone()),
                Choice::Clock(clock) => Some(clock.stored().to_string()),
                Choice::Offset(offset) => Some(offset.stored()),
            };
            match value {
                Some(value) => store(cx).set_setting(&key, &value),
                None => store(cx).remove_setting(&key),
            }
        }
    }
}

impl WorkspaceView {
    /// Opens the dropdown, or closes it when it's already open.
    pub(super) fn toggle_setting_menu(&mut self, menu: OpenMenu, cx: &mut Context<Self>) {
        self.setting_menu = if self.setting_menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    /// Closes the open dropdown, if one is open.
    pub(super) fn close_setting_menu(&mut self, cx: &mut Context<Self>) {
        if self.setting_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Saves the choice a dropdown row picked, and closes the dropdown.
    fn choose_setting(&mut self, menu: OpenMenu, choice: Choice, cx: &mut Context<Self>) {
        self.setting_menu = None;
        match save(menu.scope, menu.field, &choice, cx) {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("Could not save the setting: {err}")),
        }
        cx.notify();
    }
}

/// One dropdown: the current choice on a select, and the options under it while it's open. A click outside it
/// closes it, through the window's own handler, so the trigger and the menu stop the press from reaching there.
pub(super) fn setting_select(
    view: &WorkspaceView,
    field: Field,
    scope: Scope,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let menu = OpenMenu { field, scope };
    let open = view.setting_menu == Some(menu);
    let chosen = current(field, scope, cx);
    let shown = label(&chosen, field, scope, cx);
    let id = format!("setting-{field:?}-{scope:?}").replace(['(', ')', ' '], "");

    // Every dropdown in these windows is the same width, and its menu is as wide as it, so long labels fit.
    let trigger = components::select(id.clone(), &shown, open, theme)
        .w_full()
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_setting_menu(menu, cx)),
        );
    let mut wrapper = div()
        .relative()
        .flex_none()
        .w(px(SELECT_WIDTH))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(trigger);

    if open {
        let rows: Vec<AnyElement> = options(field, scope)
            .into_iter()
            .enumerate()
            .map(|(index, choice)| {
                let text = label(&choice, field, scope, cx);
                let selected = choice == chosen;
                components::dropdown_row(
                    format!("{id}-option-{index}"),
                    &text,
                    selected,
                    false,
                    theme,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_setting(menu, choice.clone(), cx)
                }))
                .into_any_element()
            })
            .collect();
        wrapper = wrapper.child(
            deferred(components::dropdown_menu(format!("{id}-menu"), rows, theme)).with_priority(1),
        );
    }
    wrapper
}

#[cfg(test)]
mod tests {
    use super::{Choice, Field, Scope, options};
    use looji_domain::{Clock, ProjectId};

    #[test]
    fn a_project_menu_starts_with_default_and_the_app_menu_only_does_for_the_model() {
        let project = Scope::Project(ProjectId::new());
        assert_eq!(
            options(Field::Clock, project),
            vec![
                Choice::Default,
                Choice::Clock(Clock::TwentyFour),
                Choice::Clock(Clock::Twelve),
            ]
        );
        assert_eq!(
            options(Field::Clock, Scope::App),
            vec![
                Choice::Clock(Clock::TwentyFour),
                Choice::Clock(Clock::Twelve)
            ]
        );
        assert_eq!(options(Field::Model, Scope::App)[0], Choice::Default);
    }

    #[test]
    fn the_offset_menu_lists_each_offset_once_from_west_to_east() {
        let minutes: Vec<i32> = options(Field::Offset, Scope::App)
            .into_iter()
            .filter_map(|choice| match choice {
                Choice::Offset(offset) => Some(offset.minutes()),
                _ => None,
            })
            .collect();
        assert!(minutes.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(minutes.contains(&0), "UTC is always offered");
    }
}
