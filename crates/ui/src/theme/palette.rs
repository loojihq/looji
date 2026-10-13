//! Colour roles. Components read these by name; only the values differ between modes.

use gpui::{Hsla, black, rgb};

use super::{Mode, tokens};

/// One avatar's colours: the circle and the initial drawn on it.
#[derive(Debug, Clone, Copy)]
pub struct AvatarColors {
    pub background: Hsla,
    pub text: Hsla,
}

#[derive(Debug, Clone)]
pub struct Palette {
    /// Main pane and editor background.
    pub surface: Hsla,
    /// Sidebar, cards, and dialogs.
    pub surface_raised: Hsla,
    pub surface_hover: Hsla,
    pub surface_selected: Hsla,
    pub surface_active: Hsla,
    pub title_bar: Hsla,
    pub title_bar_inactive: Hsla,
    /// Backdrop behind a dialog.
    pub scrim: Hsla,
    /// Dividers and card edges.
    pub border: Hsla,
    /// Input, button, and field outlines.
    pub border_control: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_disabled: Hsla,
    /// The one saturated colour. Primary actions and the active field outline.
    pub accent: Hsla,
    pub danger: Hsla,
    /// Text on `danger`.
    pub on_danger: Hsla,
    /// Text on `accent`.
    pub on_accent: Hsla,
    pub avatars: [AvatarColors; 6],
    /// Shadow colour used by the dialog elevation.
    pub shadow: Hsla,
}

impl Palette {
    pub fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self::dark(),
            Mode::Light => Self::light(),
        }
    }

    fn dark() -> Self {
        use tokens::dark as t;
        Self {
            surface: rgb(t::SURFACE).into(),
            surface_raised: rgb(t::SURFACE_RAISED).into(),
            surface_hover: rgb(t::SURFACE_HOVER).into(),
            surface_selected: rgb(t::SURFACE_SELECTED).into(),
            surface_active: rgb(t::SURFACE_ACTIVE).into(),
            title_bar: rgb(t::TITLE_BAR).into(),
            title_bar_inactive: rgb(t::TITLE_BAR_INACTIVE).into(),
            scrim: black().opacity(t::SCRIM_ALPHA),
            border: rgb(t::BORDER).into(),
            border_control: rgb(t::BORDER_CONTROL).into(),
            text_primary: rgb(t::TEXT_PRIMARY).into(),
            text_secondary: rgb(t::TEXT_SECONDARY).into(),
            text_disabled: rgb(t::TEXT_DISABLED).into(),
            accent: rgb(t::ACCENT).into(),
            danger: rgb(t::DANGER).into(),
            on_danger: rgb(t::ON_DANGER).into(),
            on_accent: rgb(t::ON_ACCENT).into(),
            avatars: avatars(t::AVATARS),
            shadow: black().opacity(t::SHADOW_ALPHA),
        }
    }

    fn light() -> Self {
        use tokens::light as t;
        Self {
            surface: rgb(t::SURFACE).into(),
            surface_raised: rgb(t::SURFACE_RAISED).into(),
            surface_hover: rgb(t::SURFACE_HOVER).into(),
            surface_selected: rgb(t::SURFACE_SELECTED).into(),
            surface_active: rgb(t::SURFACE_ACTIVE).into(),
            title_bar: rgb(t::TITLE_BAR).into(),
            title_bar_inactive: rgb(t::TITLE_BAR_INACTIVE).into(),
            scrim: black().opacity(t::SCRIM_ALPHA),
            border: rgb(t::BORDER).into(),
            border_control: rgb(t::BORDER_CONTROL).into(),
            text_primary: rgb(t::TEXT_PRIMARY).into(),
            text_secondary: rgb(t::TEXT_SECONDARY).into(),
            text_disabled: rgb(t::TEXT_DISABLED).into(),
            accent: rgb(t::ACCENT).into(),
            danger: rgb(t::DANGER).into(),
            on_danger: rgb(t::ON_DANGER).into(),
            on_accent: rgb(t::ON_ACCENT).into(),
            avatars: avatars(t::AVATARS),
            shadow: black().opacity(t::SHADOW_ALPHA),
        }
    }
}

fn avatars(pairs: [(u32, u32); 6]) -> [AvatarColors; 6] {
    pairs.map(|(background, text)| AvatarColors {
        background: rgb(background).into(),
        text: rgb(text).into(),
    })
}
