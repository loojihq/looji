//! Project settings: the project's working hours and its repository, in one dialog window. It opens from the
//! Settings button in the project header. A list on the left picks the section, the section fills the right, and
//! Done closes the window. Escape closes it too, and unsaved working hours ask first.

use gpui::{prelude::*, *};

use super::dialog_window::{self, Kind};
use super::{WorkspaceView, repo_dialog, schedule_dialog, setting_menu, time_settings};
use crate::{components, theme::Theme};

/// The sections the settings window lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Hours,
    Time,
    Repository,
    Ai,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Hours => "Working hours",
            Section::Time => "Time",
            Section::Repository => "Repository",
            Section::Ai => "AI",
        }
    }
}

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
        let Some(id) = self.projects.active_id() else {
            return;
        };
        self.settings_section = Section::Hours;
        self.setting_menu = None;
        self.repo_dialog = Some(repo_dialog::RepoDialog::new(id));
        self.project_settings_open = true;
        self.refresh_code_source(cx);
        self.reset_working_hours(window, cx);
        let main = cx.entity();
        cx.defer(move |cx| dialog_window::open(cx, main, Kind::ProjectSettings));
    }

    /// Reloads the working hours from the project, which discards any unsaved edit to them.
    pub(super) fn reset_working_hours(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.active() else {
            return;
        };
        let (id, schedule) = (project.id(), project.working_schedule());
        let anchor = time_settings::local_now(cx, Some(id));
        let clock = time_settings::clock_for(cx, Some(id));
        self.schedule_dialog = Some(schedule_dialog::ScheduleDialog::open(
            id, schedule, anchor, clock,
        ));
        self.schedule_focus[0].focus(window, cx);
        cx.notify();
    }

    /// Picks a section from the list on the left.
    pub(super) fn select_settings_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.settings_section = section;
        self.setting_menu = None;
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
        self.setting_menu = None;
        self.project_settings_open = false;
        cx.notify();
    }
}

/// The settings window's body: the section list on the left, and the chosen section on the right, with Done below.
pub(super) fn panel(
    view: &WorkspaceView,
    theme: &Theme,
    window: &Window,
    cx: &mut Context<WorkspaceView>,
) -> Option<AnyElement> {
    let project = view.projects.active()?.name().to_string();
    let section = view.settings_section;

    let (description, body) = match section {
        Section::Hours => (
            format!("{project}. {}", schedule_dialog::COPY),
            schedule_dialog::panel(view, theme, window, cx),
        ),
        Section::Time => (
            format!(
                "How {project} shows times. Anything not set here uses the Preferences choice."
            ),
            Some(time_section(view, theme, cx).into_any_element()),
        ),
        Section::Repository => (
            format!("The local git repository for {project}."),
            repo_dialog::panel(view, theme, cx),
        ),
        Section::Ai => (
            format!(
                "How {project} plans and answers. Anything not set here uses the Preferences choice."
            ),
            Some(ai_section(view, theme, cx).into_any_element()),
        ),
    };

    let list = [
        Section::Hours,
        Section::Time,
        Section::Repository,
        Section::Ai,
    ]
    .into_iter()
    .fold(
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .p(theme.spacing.sm)
            .w(px(168.))
            .flex_none()
            .border_r_1()
            .border_color(theme.palette.border),
        |list, item| list.child(section_item(item, item == section, theme, cx)),
    );

    let content = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .p(theme.spacing.lg)
        .child(
            div()
                .text_size(theme.typography.title)
                .font_weight(FontWeight::MEDIUM)
                .child(section.label()),
        )
        .child(
            div()
                .text_size(theme.typography.caption)
                .text_color(theme.palette.text_secondary)
                .child(description),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_setting_menu(cx)),
        )
        .children(body);

    let footer = div()
        .flex()
        .flex_row()
        .justify_end()
        .p(theme.spacing.md)
        .border_t_1()
        .border_color(theme.palette.border)
        .child(
            components::button("settings-done", "Done", components::Variant::Primary, theme)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.request_project_settings_close(window, cx)
                })),
        );

    Some(
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .child(content),
            )
            .child(footer)
            .into_any_element(),
    )
}

/// One entry in the section list. The chosen one is filled.
fn section_item(
    section: Section,
    selected: bool,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    div()
        .id(("settings-section", section as u64))
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .when(selected, |item| {
            item.bg(theme.palette.surface_selected)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.palette.text_primary)
        })
        .when(!selected, |item| {
            item.text_color(theme.palette.text_secondary)
                .hover(|style| style.bg(theme.palette.surface_hover))
        })
        .child(section.label())
        .on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_settings_section(section, cx)
            }),
        )
}

/// The project's own clock and UTC offset. Each choice is a dropdown whose first row is Default, which hands the
/// choice back to the app's setting in Preferences.
fn time_section(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> Div {
    let Some(project) = view.projects.active_id() else {
        return div();
    };
    let scope = setting_menu::Scope::Project(project);
    div()
        .flex()
        .flex_col()
        .child(components::settings_row(
            "Clock",
            Some("How times are shown and typed."),
            setting_menu::setting_select(view, setting_menu::Field::Clock, scope, theme, cx),
            theme,
        ))
        .child(components::settings_row(
            "Time zone",
            Some("A fixed offset from UTC. Working hours count from today in this offset."),
            setting_menu::setting_select(view, setting_menu::Field::Offset, scope, theme, cx),
            theme,
        ))
}

/// The project's own AI provider and model. Each choice is a dropdown whose first row is Default.
fn ai_section(view: &WorkspaceView, theme: &Theme, cx: &mut Context<WorkspaceView>) -> Div {
    let Some(project) = view.projects.active_id() else {
        return div();
    };
    let scope = setting_menu::Scope::Project(project);
    div()
        .flex()
        .flex_col()
        .child(components::settings_row(
            "Provider",
            Some("Claude Code uses the install on this computer. The Anthropic API uses the key in Preferences."),
            setting_menu::setting_select(view, setting_menu::Field::Provider, scope, theme, cx),
            theme,
        ))
        .child(components::settings_row(
            "Model",
            Some("Used for planning and task chat in this project."),
            setting_menu::setting_select(view, setting_menu::Field::Model, scope, theme, cx),
            theme,
        ))
        .child(components::settings_row(
            "Run tests in reviews",
            Some("Runs this command in the repository when a task is reviewed, and gives the reviewer the result. It runs the project's code, so it is off until you choose one."),
            setting_menu::setting_select(view, setting_menu::Field::Tests, scope, theme, cx),
            theme,
        ))
}
