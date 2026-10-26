//! The Preferences window. It holds the app-wide AI settings. A project can set its own, in its settings, and
//! everything else per project lives in the project page.

use gpui::{prelude::*, *};
use looji_ai::Provider;

use super::{WorkspaceView, ai_settings};
use crate::{components, theme::Theme};

/// The body of the Preferences window: the AI provider and model for the whole app, and the key the provider
/// needs.
pub(super) fn render(
    _view: &WorkspaceView,
    theme: &Theme,
    _window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let provider = ai_settings::app_provider(cx);
    let model = crate::settings::current(cx).selected_model;

    let mut section = div()
        .w(px(640.))
        .flex()
        .flex_col()
        .child(components::section_label("AI", theme))
        .child(
            components::settings_row(
                "Provider",
                Some(provider_note(provider)),
                components::select("pref-ai-provider-select", provider.label(), false, theme),
                theme,
            )
            .id("pref-ai-provider")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_app_provider(cx))),
        )
        .child(
            components::settings_row(
                "Model",
                Some("Used for planning and task chat. Default is the newest model."),
                components::select(
                    "pref-ai-model-select",
                    model.as_deref().unwrap_or("Default"),
                    false,
                    theme,
                ),
                theme,
            )
            .id("pref-ai-model")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_app_model(cx))),
        );

    // Only a provider that needs a key shows the key, so a provider that signs in itself has nothing to set.
    if provider.needs_key() {
        section = section.child(api_key_row(theme, cx));
    }

    div()
        .id("preferences-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(theme.spacing.lg)
        .pb(theme.spacing.lg)
        .child(section)
        .into_any_element()
}

/// What the provider needs, and whether it's ready, in one line.
fn provider_note(provider: Provider) -> &'static str {
    match provider {
        Provider::ClaudeCode if looji_ai::claude_code::installed() => {
            "Uses the Claude Code installed on this computer, with your own sign-in."
        }
        Provider::ClaudeCode => "Claude Code isn't installed here. Install it and sign in.",
        Provider::Anthropic => "Uses an API key saved on this computer.",
    }
}

/// The API key row: whether one is saved, and the buttons to save or remove it.
fn api_key_row(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let saved = looji_ai::key_store::load().ok().flatten().is_some();
    let mut buttons = div().flex().flex_row().gap(theme.spacing.sm).child(
        components::button(
            "pref-key-paste",
            "Paste key",
            components::Variant::Secondary,
            theme,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.paste_api_key(cx))),
    );
    if saved {
        buttons = buttons.child(
            components::button(
                "pref-key-remove",
                "Remove",
                components::Variant::Ghost,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.remove_api_key(cx))),
        );
    }
    components::settings_row(
        "API key",
        Some(if saved {
            "Saved in the system's key store."
        } else {
            "Not set. Copy a key, then paste it here."
        }),
        buttons,
        theme,
    )
}

impl WorkspaceView {
    /// Opens Preferences in its own window. Only one is open, so a second press does nothing.
    pub(super) fn open_preferences(&mut self, cx: &mut gpui::Context<Self>) {
        if self.preferences_open {
            return;
        }
        self.preferences_open = true;
        let main = cx.entity();
        cx.defer(move |cx| {
            super::dialog_window::open(cx, main, super::dialog_window::Kind::Preferences)
        });
        cx.notify();
    }

    /// Marks the Preferences window closed. Its window closes itself once this has run.
    pub(super) fn close_preferences(&mut self, cx: &mut gpui::Context<Self>) {
        if self.preferences_open {
            self.preferences_open = false;
            cx.notify();
        }
    }

    /// Saves the API key on the clipboard in the system's key store.
    pub(super) fn paste_api_key(&mut self, cx: &mut gpui::Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let Some(key) = text
            .as_deref()
            .and_then(|text| looji_ai::ApiKey::new(text.trim()))
        else {
            self.status = Some("Copy an API key first, then paste it here.".to_string());
            return cx.notify();
        };
        self.status = Some(match looji_ai::key_store::save(&key) {
            Ok(()) => "The API key is saved.".to_string(),
            Err(err) => format!("Could not save the API key: {err}"),
        });
        cx.notify();
    }

    /// Removes the saved API key.
    pub(super) fn remove_api_key(&mut self, cx: &mut gpui::Context<Self>) {
        self.status = Some(match looji_ai::key_store::delete() {
            Ok(()) => "The API key is removed.".to_string(),
            Err(err) => format!("Could not remove the API key: {err}"),
        });
        cx.notify();
    }
}
