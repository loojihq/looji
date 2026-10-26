//! The icons the app draws, from Tabler Icons (MIT, see `assets/icons/LICENSE.txt`). They are
//! compiled in, so the program needs no files beside it. Draw one with `components::icon`.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Each icon by the path `components::icon` asks for.
const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/paperclip.svg",
        include_bytes!("../assets/icons/paperclip.svg"),
    ),
    (
        "icons/message-ai.svg",
        include_bytes!("../assets/icons/message-ai.svg"),
    ),
    (
        "icons/robot.svg",
        include_bytes!("../assets/icons/robot.svg"),
    ),
    (
        "icons/alert-triangle.svg",
        include_bytes!("../assets/icons/alert-triangle.svg"),
    ),
    (
        "icons/arrow-down.svg",
        include_bytes!("../assets/icons/arrow-down.svg"),
    ),
    (
        "icons/arrow-up-right.svg",
        include_bytes!("../assets/icons/arrow-up-right.svg"),
    ),
    (
        "icons/arrow-up.svg",
        include_bytes!("../assets/icons/arrow-up.svg"),
    ),
    (
        "icons/calendar-week.svg",
        include_bytes!("../assets/icons/calendar-week.svg"),
    ),
    (
        "icons/calendar.svg",
        include_bytes!("../assets/icons/calendar.svg"),
    ),
    (
        "icons/chart-bar.svg",
        include_bytes!("../assets/icons/chart-bar.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/check.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/circle-check.svg",
        include_bytes!("../assets/icons/circle-check.svg"),
    ),
    (
        "icons/circle.svg",
        include_bytes!("../assets/icons/circle.svg"),
    ),
    (
        "icons/clock.svg",
        include_bytes!("../assets/icons/clock.svg"),
    ),
    ("icons/copy.svg", include_bytes!("../assets/icons/copy.svg")),
    ("icons/dots.svg", include_bytes!("../assets/icons/dots.svg")),
    (
        "icons/download.svg",
        include_bytes!("../assets/icons/download.svg"),
    ),
    ("icons/edit.svg", include_bytes!("../assets/icons/edit.svg")),
    (
        "icons/file-text.svg",
        include_bytes!("../assets/icons/file-text.svg"),
    ),
    ("icons/flag.svg", include_bytes!("../assets/icons/flag.svg")),
    (
        "icons/folder.svg",
        include_bytes!("../assets/icons/folder.svg"),
    ),
    (
        "icons/git-branch.svg",
        include_bytes!("../assets/icons/git-branch.svg"),
    ),
    (
        "icons/grip-vertical.svg",
        include_bytes!("../assets/icons/grip-vertical.svg"),
    ),
    (
        "icons/layout-kanban.svg",
        include_bytes!("../assets/icons/layout-kanban.svg"),
    ),
    ("icons/link.svg", include_bytes!("../assets/icons/link.svg")),
    ("icons/list.svg", include_bytes!("../assets/icons/list.svg")),
    ("icons/moon.svg", include_bytes!("../assets/icons/moon.svg")),
    (
        "icons/photo.svg",
        include_bytes!("../assets/icons/photo.svg"),
    ),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    (
        "icons/route.svg",
        include_bytes!("../assets/icons/route.svg"),
    ),
    (
        "icons/search.svg",
        include_bytes!("../assets/icons/search.svg"),
    ),
    (
        "icons/selector.svg",
        include_bytes!("../assets/icons/selector.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../assets/icons/settings.svg"),
    ),
    (
        "icons/sparkles.svg",
        include_bytes!("../assets/icons/sparkles.svg"),
    ),
    ("icons/sun.svg", include_bytes!("../assets/icons/sun.svg")),
    (
        "icons/trash.svg",
        include_bytes!("../assets/icons/trash.svg"),
    ),
    (
        "icons/users.svg",
        include_bytes!("../assets/icons/users.svg"),
    ),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
