//! Application-wide colors, stored as a gpui global and read via [`ActiveTheme`].
//!
//! To add a theme, create a file next to `one_dark.rs` with its own constructor.

mod one_dark;

use gpui::{App, Global, Hsla};

pub struct Theme {
    pub title_bar_background: Hsla,
    pub title_bar_inactive_background: Hsla,
    pub editor_background: Hsla,
    pub text: Hsla,
    pub ghost_element_hover: Hsla,
    pub ghost_element_active: Hsla,
}

impl Global for Theme {}

pub fn init(cx: &mut App) {
    cx.set_global(Theme::one_dark());
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}
