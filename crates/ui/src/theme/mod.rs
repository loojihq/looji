//! Design tokens: colour, type, spacing, radius, and elevation, in one place.
//!
//! Components read `cx.theme()` and use the named roles. They don't spell out colours or sizes.
//! `tokens.rs` holds the raw values, `palette.rs` maps them to roles, and `contrast.rs` and
//! `guard.rs` are tests that check the contrast of every role pair and that no component
//! hard-codes a colour.

mod palette;
mod tokens;

#[cfg(test)]
mod contrast;
#[cfg(test)]
mod guard;

pub use palette::{AvatarColors, Palette};

use gpui::{App, BoxShadow, Global, Pixels, point, px};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl From<looji_domain::PreferredTheme> for Mode {
    fn from(theme: looji_domain::PreferredTheme) -> Self {
        match theme {
            looji_domain::PreferredTheme::Light => Mode::Light,
            looji_domain::PreferredTheme::Dark => Mode::Dark,
        }
    }
}

impl Mode {
    pub fn toggled(self) -> Self {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Typography {
    pub caption: Pixels,
    pub body: Pixels,
    pub title: Pixels,
    pub heading: Pixels,
}

#[derive(Debug, Clone, Copy)]
pub struct Spacing {
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
}

#[derive(Debug, Clone, Copy)]
pub struct Radius {
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
}

/// Surfaces step up in colour before shadows are used. Only dialogs cast a shadow.
#[derive(Debug, Clone)]
pub struct Elevation {
    pub overlay: Vec<BoxShadow>,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub mode: Mode,
    pub palette: Palette,
    pub typography: Typography,
    pub spacing: Spacing,
    pub radius: Radius,
    pub elevation: Elevation,
}

impl Theme {
    pub fn for_mode(mode: Mode) -> Self {
        Self {
            mode,
            palette: Palette::for_mode(mode),
            typography: Typography {
                caption: px(tokens::typography::CAPTION),
                body: px(tokens::typography::BODY),
                title: px(tokens::typography::TITLE),
                heading: px(tokens::typography::HEADING),
            },
            spacing: Spacing {
                xs: px(tokens::spacing::XS),
                sm: px(tokens::spacing::SM),
                md: px(tokens::spacing::MD),
                lg: px(tokens::spacing::LG),
            },
            radius: Radius {
                sm: px(tokens::radius::SM),
                md: px(tokens::radius::MD),
                lg: px(tokens::radius::LG),
            },
            elevation: Elevation {
                overlay: vec![BoxShadow {
                    color: Palette::for_mode(mode).shadow,
                    offset: point(px(0.), px(tokens::shadow::OFFSET_Y)),
                    blur_radius: px(tokens::shadow::BLUR),
                    spread_radius: px(0.),
                    inset: false,
                }],
            },
        }
    }
}

impl Global for Theme {}

pub fn init(cx: &mut App, mode: Mode) {
    cx.set_global(Theme::for_mode(mode));
}

/// Switches mode and re-renders every window, so nothing keeps the old palette.
pub fn set_mode(mode: Mode, cx: &mut App) {
    cx.set_global(Theme::for_mode(mode));
    cx.refresh_windows();
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

impl From<Mode> for looji_domain::PreferredTheme {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Light => looji_domain::PreferredTheme::Light,
            Mode::Dark => looji_domain::PreferredTheme::Dark,
        }
    }
}
