//! The Preferences screen: appearance, updates, the board, the assistant, this workspace, and help.
//! Each toggle and select saves through `settings::update`, so the choice is kept after a restart.

use gpui::{prelude::*, *};
use looji_domain::{DefaultView, Preference, SettingsRepository, Status, UpdateChannel};

use super::{WorkspaceView, schedule_dialog};
use crate::{
    components, settings,
    theme::{Mode, Theme},
};

/// The body of the Preferences screen, under its header.
pub(super) fn render(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let prefs = settings::current(cx);
    // Copied out first, so each control's click handler owns its own value.
    let default_view = prefs.default_view;
    let model = prefs.selected_model.clone();
    let dark = theme.mode == Mode::Dark;
    let due_today_on = prefs.notify_due_today;
    let overdue_on = prefs.notify_overdue;
    let beta = prefs.update_channel == UpdateChannel::Beta;
    let workspace_id = view.list.active_workspace_id();
    let workspace_name = view.list.active().name().to_string();
    let in_review = view
        .projects
        .active()
        .map(|project| project.is_status_visible(Status::InReview));

    let board_row: AnyElement = match in_review {
        Some(shown) => components::settings_row(
            "In review column",
            Some("Show the In review column on this project's board."),
            components::toggle(shown, theme),
            theme,
        )
        .id("pref-in-review")
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.request_column_visibility(Status::InReview, !shown, cx);
        }))
        .into_any_element(),
        None => components::settings_row(
            "In review column",
            Some("Open a project to change its board."),
            div()
                .text_color(theme.palette.text_disabled)
                .child("No project"),
            theme,
        )
        .into_any_element(),
    };

    // The hours form opens under its setting, and closes the same way.
    let hours = schedule_dialog::panel(view, theme, window, cx);

    let body = div()
        .id("preferences-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(theme.spacing.lg)
        .pb(theme.spacing.lg)
        .child(
            div()
                .w(px(640.))
                .flex()
                .flex_col()
                .child(components::section_label("Appearance", theme))
                .child(
                    components::settings_row(
                        "Dark mode",
                        Some("Use dark surfaces across the app."),
                        components::toggle(dark, theme),
                        theme,
                    )
                    .id("pref-dark-mode")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_theme(cx))),
                )
                .child(components::section_label("Updates", theme))
                .child(
                    components::settings_row(
                        "Beta updates",
                        Some("Get preview releases before they are stable."),
                        components::toggle(beta, theme),
                        theme,
                    )
                    .id("pref-beta")
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            let next = if beta {
                                UpdateChannel::Stable
                            } else {
                                UpdateChannel::Beta
                            };
                            this.save_preference(Preference::UpdateChannel(next), cx);
                        },
                    )),
                )
                .child(components::section_label("Notifications", theme))
                .child(
                    components::settings_row(
                        "Due today",
                        Some("Count tasks due today next to Today in the sidebar."),
                        components::toggle(due_today_on, theme),
                        theme,
                    )
                    .id("pref-notify-due-today")
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.save_preference(Preference::NotifyDueToday(!due_today_on), cx);
                        },
                    )),
                )
                .child(
                    components::settings_row(
                        "Overdue tasks",
                        Some("Count overdue tasks next to Today in the sidebar."),
                        components::toggle(overdue_on, theme),
                        theme,
                    )
                    .id("pref-notify-overdue")
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.save_preference(Preference::NotifyOverdue(!overdue_on), cx);
                        },
                    )),
                )
                .child(components::section_label("Board", theme))
                .child(
                    components::settings_row(
                        "Default view",
                        Some("The view a project opens in when it has no saved view."),
                        components::select(
                            "pref-default-view-select",
                            default_view_label(default_view),
                            false,
                            theme,
                        ),
                        theme,
                    )
                    .id("pref-default-view")
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            let next = next_view(default_view);
                            this.save_preference(Preference::DefaultView(next), cx);
                        },
                    )),
                )
                .child(board_row)
                .children(hours)
                .child(components::section_label("AI", theme))
                .child(ai_rows(theme, cx))
                .child(
                    components::settings_row(
                        "Model",
                        Some("The model that plans projects and answers task chat."),
                        components::select(
                            "pref-model-select",
                            model.as_deref().unwrap_or("Default"),
                            false,
                            theme,
                        ),
                        theme,
                    )
                    .id("pref-model")
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            let next = settings::next_model(model.as_deref());
                            this.save_preference(Preference::SelectedModel(next), cx);
                        },
                    )),
                )
                .child(components::section_label("This workspace", theme))
                .child(components::settings_row(
                    workspace_name.as_str(),
                    Some("Its icon, its name, and whether it can be deleted."),
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.spacing.sm)
                        .child(
                            components::button(
                                "pref-workspace-icon",
                                "Icon",
                                components::Variant::Secondary,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.choose_icon(workspace_id, cx);
                                },
                            )),
                        )
                        .child(
                            components::button(
                                "pref-workspace-rename",
                                "Rename",
                                components::Variant::Secondary,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.begin_rename(workspace_id, window, cx);
                                    this.open_screen(super::screen::Screen::Workspace, cx);
                                },
                            )),
                        )
                        .child(
                            components::button(
                                "pref-workspace-delete",
                                "Delete",
                                components::Variant::Danger,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    this.begin_delete(workspace_id, window, cx);
                                },
                            )),
                        ),
                    theme,
                ))
                .child(components::section_label("Help", theme))
                .child(components::settings_row(
                    "Welcome tour",
                    Some("Walk through the main parts of the app again."),
                    components::button(
                        "pref-tour",
                        "Show again",
                        components::Variant::Secondary,
                        theme,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.replay_tour(cx))),
                    theme,
                )),
        );

    body.into_any_element()
}

fn default_view_label(view: DefaultView) -> &'static str {
    match view {
        DefaultView::Board => "Board",
        DefaultView::List => "List",
        DefaultView::Roadmap => "Roadmap",
        DefaultView::Workload => "Workload",
    }
}

fn next_view(view: DefaultView) -> DefaultView {
    match view {
        DefaultView::Board => DefaultView::List,
        DefaultView::List => DefaultView::Roadmap,
        DefaultView::Roadmap => DefaultView::Workload,
        DefaultView::Workload => DefaultView::Board,
    }
}

impl WorkspaceView {
    /// Saves one preference and says so if the save fails. The screen re-renders with the new value.
    pub(super) fn save_preference(&mut self, change: Preference, cx: &mut Context<Self>) {
        if let Err(err) = settings::update(cx, change) {
            self.status = Some(format!("Could not save the setting: {err}"));
        }
        cx.notify();
    }
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
}

/// The setting that holds the AI provider's name.
const AI_PROVIDER_KEY: &str = "ai_provider";

/// The AI provider the settings choose. Claude Code when nothing has been chosen.
pub(super) fn ai_provider(cx: &gpui::App) -> looji_ai::Provider {
    super::store(cx)
        .get_setting(AI_PROVIDER_KEY)
        .ok()
        .flatten()
        .as_deref()
        .and_then(looji_ai::Provider::from_stored)
        .unwrap_or(looji_ai::Provider::ClaudeCode)
}

/// The model for a plan: the one the settings choose, or the default.
pub(super) fn plan_model(cx: &gpui::App) -> String {
    settings::current(cx)
        .selected_model
        .unwrap_or_else(|| looji_ai::models::DEFAULT_MODEL.to_string())
}

impl WorkspaceView {
    /// Moves the AI provider to the next choice and saves it.
    pub(super) fn cycle_ai_provider(&mut self, cx: &mut gpui::Context<Self>) {
        let current = ai_provider(cx);
        let index = looji_ai::Provider::ALL
            .iter()
            .position(|provider| *provider == current)
            .unwrap_or(0);
        let next = looji_ai::Provider::ALL[(index + 1) % looji_ai::Provider::ALL.len()];
        match super::store(cx).set_setting(AI_PROVIDER_KEY, next.stored()) {
            Ok(()) => self.status = None,
            Err(err) => self.status = Some(format!("Could not save the AI provider: {err}")),
        }
        cx.notify();
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

/// The AI settings: which provider answers, whether Claude Code is installed, and the saved API key.
fn ai_rows(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let provider = ai_provider(cx);
    let claude_found = looji_ai::claude_code::installed();
    let key_saved = looji_ai::key_store::load().ok().flatten().is_some();

    let mut key_buttons = div().flex().flex_row().gap(theme.spacing.sm).child(
        components::button(
            "pref-key-paste",
            "Paste key",
            components::Variant::Secondary,
            theme,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.paste_api_key(cx))),
    );
    if key_saved {
        key_buttons = key_buttons.child(
            components::button(
                "pref-key-remove",
                "Remove",
                components::Variant::Ghost,
                theme,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.remove_api_key(cx))),
        );
    }

    let (claude_note, claude_state, claude_colour) = if claude_found {
        (
            "Installed on this computer. It uses your own sign-in.",
            "Ready",
            theme.palette.text_primary,
        )
    } else {
        (
            "Not installed. Install Claude Code and sign in, then choose it above.",
            "Missing",
            theme.palette.danger,
        )
    };

    div()
        .flex()
        .flex_col()
        .child(
            components::settings_row(
                "AI provider",
                Some("Claude Code uses your own sign-in. The API key option uses the key saved below."),
                components::select(
                    "pref-ai-provider-select",
                    provider.label(),
                    false,
                    theme,
                ),
                theme,
            )
            .id("pref-ai-provider")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_ai_provider(cx))),
        )
        .child(components::settings_row(
            "Claude Code",
            Some(claude_note),
            div()
                .text_color(claude_colour)
                .child(claude_state),
            theme,
        ))
        .child(components::settings_row(
            "Anthropic API key",
            Some(if key_saved {
                "A key is saved in the system's key store."
            } else {
                "No key is saved. Copy a key, then paste it here."
            }),
            key_buttons,
            theme,
        ))
}
