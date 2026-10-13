//! Preference rows in the sidebar. Each click moves one setting to its next value and saves it.

use gpui::{prelude::*, *};
use looji_domain::{DefaultView, Preference, Preferences, UpdateChannel};

use super::WorkspaceView;
use crate::{settings, theme::Theme};

#[derive(Debug, Clone, Copy)]
pub(super) enum Setting {
    View,
    Channel,
    Model,
}

pub(super) fn section(
    prefs: &Preferences,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .mt(theme.spacing.md)
        .child(
            div()
                .px(theme.spacing.sm)
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child("PREFERENCES"),
        )
        .child(row(
            "Default view",
            prefs.default_view.as_str().to_string(),
            Setting::View,
            theme,
            cx,
        ))
        .child(row(
            "Update channel",
            prefs.update_channel.as_str().to_string(),
            Setting::Channel,
            theme,
            cx,
        ))
        .child(row(
            "Model",
            prefs
                .selected_model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            Setting::Model,
            theme,
            cx,
        ))
}

fn row(
    label: &'static str,
    value: String,
    which: Setting,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(label)
        .flex()
        .flex_row()
        .justify_between()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .text_size(theme.typography.body)
        .hover(|style| style.bg(theme.palette.surface_hover))
        .child(label)
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.palette.text_secondary)
                .child(value),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.cycle_preference(which, cx)))
}

fn next_view(view: DefaultView) -> DefaultView {
    match view {
        DefaultView::Board => DefaultView::List,
        DefaultView::List => DefaultView::Roadmap,
        DefaultView::Roadmap => DefaultView::Workload,
        DefaultView::Workload => DefaultView::Board,
    }
}

fn next_channel(channel: UpdateChannel) -> UpdateChannel {
    match channel {
        UpdateChannel::Stable => UpdateChannel::Beta,
        UpdateChannel::Beta => UpdateChannel::Stable,
    }
}

impl WorkspaceView {
    pub(super) fn cycle_preference(&mut self, which: Setting, cx: &mut Context<Self>) {
        let prefs = settings::current(cx);
        let change = match which {
            Setting::View => Preference::DefaultView(next_view(prefs.default_view)),
            Setting::Channel => Preference::UpdateChannel(next_channel(prefs.update_channel)),
            Setting::Model => {
                Preference::SelectedModel(settings::next_model(prefs.selected_model.as_deref()))
            }
        };
        if let Err(err) = settings::update(cx, change) {
            self.status = Some(format!("Could not save the setting: {err}"));
        }
        cx.notify();
    }
}
