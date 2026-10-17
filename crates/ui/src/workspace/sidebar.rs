//! The sidebar. Every entry is a single row: a workspace to switch to, a screen to open, or a project.
//! Workspace settings (icon, rename, delete) and the app's preferences live on the Preferences screen.

use std::path::Path;

use gpui::{prelude::*, *};
use looji_domain::Workspace;

use super::screen::Screen;
use super::{Rename, WorkspaceView, avatar, projects};
use crate::{
    components,
    database::AppDatabase,
    settings,
    theme::{ActiveTheme, Theme},
};

const SIDEBAR_WIDTH: Pixels = px(224.);

pub fn render(view: &WorkspaceView, cx: &mut Context<WorkspaceView>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let active = view.list.active_workspace_id();
    let icons_dir = cx.global::<AppDatabase>().icons_dir.clone();

    let rows: Vec<AnyElement> = view
        .list
        .items()
        .iter()
        .map(|workspace| {
            let id = workspace.id();
            match &view.rename {
                Some(rename) if rename.id == id => editing_row(rename, &theme).into_any_element(),
                _ => workspace_row(workspace, id == active, &icons_dir, &theme, cx)
                    .into_any_element(),
            }
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(theme.spacing.xs)
        .w(SIDEBAR_WIDTH)
        .h_full()
        .p(theme.spacing.sm)
        .bg(theme.palette.surface_raised)
        .border_r_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.body)
        .children(rows)
        .child(single_item(
            SidebarItem {
                id: "new-workspace",
                icon: "plus",
                label: "New workspace",
                trailing: None,
            },
            |this, window, cx| this.create_workspace(window, cx),
            &theme,
            cx,
        ))
        .child(
            div()
                .mt(theme.spacing.sm)
                .flex()
                .flex_col()
                .gap(theme.spacing.xs)
                .child(single_item(
                    SidebarItem {
                        id: "sidebar-search",
                        icon: "search",
                        label: "Search",
                        trailing: Some("Ctrl F"),
                    },
                    |this, window, cx| this.focus_search(window, cx),
                    &theme,
                    cx,
                ))
                .child(screen_item(Screen::Today, "sun", "Today", view, &theme, cx))
                .child(screen_item(
                    Screen::Week,
                    "calendar-week",
                    "Week",
                    view,
                    &theme,
                    cx,
                )),
        )
        .child(projects::sidebar_section(view, &theme, cx))
        .child(div().flex_1())
        .child(screen_item(
            Screen::Preferences,
            "settings",
            "Preferences",
            view,
            &theme,
            cx,
        ))
        .when_some(view.status.as_ref(), |sidebar, message| {
            sidebar.child(
                div()
                    .px(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message.clone()),
            )
        })
}

/// One workspace. A click switches to it, and a double-click starts renaming it.
fn workspace_row(
    workspace: &Workspace,
    is_active: bool,
    icons_dir: &Path,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id = workspace.id();
    div()
        .id(id.as_uuid())
        .flex()
        .flex_row()
        .items_center()
        .gap(theme.spacing.sm)
        .px(theme.spacing.sm)
        .py(theme.spacing.xs)
        .rounded(theme.radius.md)
        .cursor_pointer()
        .min_w_0()
        .when(is_active, |row| {
            row.bg(theme.palette.surface_selected)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!is_active, |row| {
            row.hover(|style| style.bg(theme.palette.surface_hover))
        })
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.begin_rename(id, window, cx);
            } else {
                this.switch_to(id, cx);
            }
        }))
        .child(avatar::render(workspace, icons_dir, theme))
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(workspace.name().to_string()),
        )
}

/// The row being renamed. Keys are handled by the view; clicks here stay in the field.
fn editing_row(rename: &Rename, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.xs)
        .child(
            div()
                .px(theme.spacing.sm)
                .py(theme.spacing.xs)
                .rounded(theme.radius.md)
                .bg(theme.palette.surface_selected)
                .border_1()
                .border_color(theme.palette.border)
                .child(format!("{}|", rename.draft)),
        )
        .when_some(rename.error.as_ref(), |row, message| {
            row.child(
                div()
                    .px(theme.spacing.sm)
                    .text_size(theme.typography.caption)
                    .text_color(theme.palette.danger)
                    .child(message.clone()),
            )
        })
}

/// A sidebar entry that opens a workspace screen. It is highlighted while that screen is showing.
fn screen_item(
    screen: Screen,
    icon: &str,
    label: &'static str,
    view: &WorkspaceView,
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    let id: &'static str = match screen {
        Screen::Today => "sidebar-today",
        Screen::Week => "sidebar-week",
        Screen::Preferences => "sidebar-preferences",
    };
    // Today shows how many tasks need attention, as the notification settings choose.
    let count = (screen == Screen::Today)
        .then(|| view.attention_count(&settings::current(cx)))
        .filter(|count| *count > 0)
        .map(|count| count.to_string());
    components::nav_item(
        id,
        icon,
        label,
        view.screen == Some(screen),
        count.as_deref(),
        theme,
    )
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_screen(screen, cx)))
}

/// What one sidebar entry shows: its id, icon, label, and an optional trailing hint such as a shortcut.
struct SidebarItem {
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    trailing: Option<&'static str>,
}

/// A sidebar entry that runs an action when clicked. The action gets the window, for the dialogs it opens.
fn single_item(
    item: SidebarItem,
    run: fn(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>),
    theme: &Theme,
    cx: &mut Context<WorkspaceView>,
) -> impl IntoElement {
    components::nav_item(item.id, item.icon, item.label, false, item.trailing, theme)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| run(this, window, cx)))
}
