use gpui::rgb;

use super::Theme;

impl Theme {
    /// Zed's default "One Dark" theme (assets/themes/one/one.json).
    pub fn one_dark() -> Self {
        Self {
            title_bar_background: rgb(0x3b414d).into(),
            title_bar_inactive_background: rgb(0x2e343e).into(),
            editor_background: rgb(0x282c33).into(),
            panel_background: rgb(0x2f343e).into(),
            border: rgb(0x464b57).into(),
            text: rgb(0xdce0e5).into(),
            ghost_element_hover: rgb(0x363c46).into(),
            ghost_element_active: rgb(0x454a56).into(),
            element_selected: rgb(0x454a56).into(),
            error: rgb(0xd07277).into(),
            // TEMPORARY: One Dark's accent colours; replace with design-system tokens.
            avatar_colors: [
                rgb(0x74ade8).into(),
                rgb(0xa1c181).into(),
                rgb(0xdec184).into(),
                rgb(0xd07277).into(),
                rgb(0xb477cf).into(),
                rgb(0x6fb3c0).into(),
            ],
        }
    }
}
