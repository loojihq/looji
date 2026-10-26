//! Project settings: the project's working hours and its repository, in one dialog window. It opens from the
//! Settings button in the project header. Escape closes it, and unsaved working hours ask first.

use gpui::{prelude::*, *};

use super::dialog_window::{self, Kind};
use super::{WorkspaceView, repo_dialog, schedule_dialog};
use crate::{components, theme::Theme};

/// The Settings button in the project header.
pub(super) fn button(theme: &Theme, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    components::button_icon(
        "project-settings",
        "settings",
        "Settings",
        components::Variant::Secondary,
        theme,
    )
    .on_click(
        cx.listener(|this, _: &ClickEvent, window, cx| this.open_project_settings(window, cx)),
    )
}

impl WorkspaceView {
    /// Opens the project's settings in a dialog window. Only one is open, so a second press does nothing.
    pub(super) fn open_project_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_settings_open {
            return;
        }
        let Some(project) = self.projects.active() else {
            return;
        };
        let (id, schedule) = (project.id(), project.working_schedule());
        let anchor = looji_domain::LocalMoment::from_timestamp_utc(looji_domain::Timestamp::now());
        self.schedule_dialog = Some(schedule_dialog::ScheduleDialog::open(id, schedule, anchor));
        self.repo_dialog = Some(repo_dialog::RepoDialog::new(id));
        self.project_settings_open = true;
        self.refresh_code_source(cx);
        let main = cx.entity();
        cx.defer(move |cx| dialog_window::open(cx, main, Kind::ProjectSettings));
        self.schedule_focus[0].focus(window, cx);
        cx.notify();
    }

    /// Closes the settings. Unsaved working hours keep the dialog open and ask first, as the hours form does.
    pub(super) fn request_project_settings_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .schedule_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.dirty())
        {
            return self.request_schedule_close(window, cx);
        }
        self.schedule_dialog = None;
        self.repo_dialog = None;
        self.project_settings_open = false;
        cx.notify();
    }
}

/// The settings body: the working hours, then the repository, each under its own heading.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let hours = schedule_dialog::panel(view, theme, window, cx);
    let repository = repo_dialog::panel(view, theme, cx);
    Some(
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.lg)
            .p(theme.spacing.lg)
            .children(hours)
            .child(components::section_label("Repository", theme))
            .children(repository)
            .into_any_element(),
    )
}
