//! The Preferences screen: appearance, updates, the board, the assistant, this workspace, and help.
//! Each toggle and select saves through `settings::update`, so the choice is kept after a restart.

use gpui::{prelude::*, *};
use looji_domain::{DefaultView, Preference, Status, UpdateChannel};

use super::{WorkspaceView, columns, schedule_dialog};
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

    // The column and hours confirmations open under their settings, and close the same way.
    let column = view
        .pending_column
        .as_ref()
        .map(|pending| columns::dialog(pending, theme, cx));
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
                .children(column)
                .children(hours)
                .child(components::section_label("Assistant", theme))
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
