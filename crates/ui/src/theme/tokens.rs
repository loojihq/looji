//! Raw token values. This is the only file that spells out colours, sizes, and radii for the UI.
//!
//! The colours are the Gruvbox palette from the JLC player: Dark Hard for dark mode and Light Hard for
//! light mode. Each value is named for the role JLC gives it, so the two apps read alike. Two values
//! differ from JLC so the contrast rules in `contrast.rs` hold: the light-mode accent is Gruvbox's dark
//! orange, because the bright orange is too faint on a light background, and the light-mode danger is
//! Gruvbox's dark red for the same reason. `palette.rs` gives these values their roles, and components
//! only ever read roles.

/// Dark mode: Gruvbox Dark Hard.
pub mod dark {
    /// The window behind everything (JLC's `background`).
    pub const SURFACE: u32 = 0x1d2021;
    /// Sidebars, columns, and dialogs (JLC's `controls_background`).
    pub const SURFACE_RAISED: u32 = 0x282828;
    /// Hovered rows and buttons (JLC's `hover`).
    pub const SURFACE_HOVER: u32 = 0x3c3836;
    /// The selected row, a step above hover so it reads as chosen.
    pub const SURFACE_SELECTED: u32 = 0x454241;
    /// A pressed control.
    pub const SURFACE_ACTIVE: u32 = 0x665c54;
    /// The title bar (JLC's `top_bar_background`). The inactive window keeps the same colour.
    pub const TITLE_BAR: u32 = 0x1d2021;
    pub const TITLE_BAR_INACTIVE: u32 = 0x1d2021;
    /// Dividers and card edges (JLC's `divider`). Decorative, so not held to the 3:1 control rule.
    pub const BORDER: u32 = 0x3c3836;
    /// Input, button, and field outlines. Held to 3:1 against their surface.
    pub const BORDER_CONTROL: u32 = 0xa89984;
    /// The brightest text (JLC's `fg_heading`).
    pub const TEXT_PRIMARY: u32 = 0xebdbb2;
    /// Main text (JLC's `fg_primary`).
    pub const TEXT_SECONDARY: u32 = 0xbdae93;
    /// Disabled text (JLC's `fg_secondary`), light enough to clear 4.5:1 on every surface.
    pub const TEXT_DISABLED: u32 = 0xa89984;
    /// JLC's default accent swatch, Gruvbox orange.
    pub const ACCENT: u32 = 0xfe8019;
    /// Gruvbox red.
    pub const DANGER: u32 = 0xff7b6e;
    /// Text on the danger colour (JLC's `accent_text`).
    pub const ON_DANGER: u32 = 0x1d2021;
    /// Text on the accent colour (JLC's `accent_text`).
    pub const ON_ACCENT: u32 = 0x1d2021;
    /// Overlay colour, black at this alpha. Drawn behind dialogs.
    pub const SCRIM_ALPHA: f32 = 0.5;
    /// Shadow colour for the dialog elevation, black at this alpha.
    pub const SHADOW_ALPHA: f32 = 0.35;
    /// Avatar pairs, as (background, text), from JLC's accent swatches. Each pair must clear 4.5:1.
    pub const AVATARS: [(u32, u32); 6] = [
        (0xfe8019, ON_ACCENT),
        (0xb8bb26, ON_ACCENT),
        (0x83a598, ON_ACCENT),
        (0xd3869b, ON_ACCENT),
        (0x8ec07c, ON_ACCENT),
        (0xfabd2f, ON_ACCENT),
    ];
    /// The close button's hover colour: Windows' standard close red, as JLC uses it.
    pub const CAPTION_CLOSE: u32 = 0xe81120;
    pub const ON_CAPTION_CLOSE: u32 = 0xffffff;
}

/// Light mode: Gruvbox Light Hard.
pub mod light {
    /// The window behind everything (JLC's `background`).
    pub const SURFACE: u32 = 0xf9f5d7;
    /// Sidebars, columns, and dialogs (JLC's `controls_background`).
    pub const SURFACE_RAISED: u32 = 0xebdbb2;
    /// Hovered rows and buttons (JLC's `hover`).
    pub const SURFACE_HOVER: u32 = 0xd5c4a1;
    /// The selected row, a step below hover so it reads as chosen.
    pub const SURFACE_SELECTED: u32 = 0xc9b895;
    /// A pressed control.
    pub const SURFACE_ACTIVE: u32 = 0xa89984;
    /// The title bar (JLC's `top_bar_background`). The inactive window keeps the same colour.
    pub const TITLE_BAR: u32 = 0xf9f5d7;
    pub const TITLE_BAR_INACTIVE: u32 = 0xf9f5d7;
    /// Dividers and card edges (JLC's `divider`). Decorative, so not held to the 3:1 control rule.
    pub const BORDER: u32 = 0xbdae93;
    /// Input, button, and field outlines. Held to 3:1 against their surface.
    pub const BORDER_CONTROL: u32 = 0x504945;
    /// The brightest text (JLC's `fg_heading`).
    pub const TEXT_PRIMARY: u32 = 0x282828;
    /// Main text (JLC's `fg_secondary`).
    pub const TEXT_SECONDARY: u32 = 0x504945;
    /// Disabled text, darker than JLC's so it clears 4.5:1 on the raised surface too.
    pub const TEXT_DISABLED: u32 = 0x665c54;
    /// Gruvbox dark orange. JLC's bright orange is too faint on a light surface, so this is the
    /// same hue at a contrast the rules allow.
    pub const ACCENT: u32 = 0xaf3a03;
    /// Gruvbox dark red, for the same reason as the accent.
    pub const DANGER: u32 = 0x9d0006;
    pub const ON_DANGER: u32 = 0xfbf1c7;
    pub const ON_ACCENT: u32 = 0xfbf1c7;
    pub const SCRIM_ALPHA: f32 = 0.35;
    pub const SHADOW_ALPHA: f32 = 0.18;
    /// Avatar pairs, as (background, text), from Gruvbox's light accents. Each pair must clear 4.5:1.
    pub const AVATARS: [(u32, u32); 6] = [
        (0xaf3a03, ON_ACCENT),
        (0x5f5a0b, ON_ACCENT),
        (0x076678, ON_ACCENT),
        (0x8f3f71, ON_ACCENT),
        (0x2f5f4a, ON_ACCENT),
        (0x8f5f0c, ON_ACCENT),
    ];
    /// The close button's hover colour: Windows' standard close red, as JLC uses it.
    pub const CAPTION_CLOSE: u32 = 0xe81120;
    pub const ON_CAPTION_CLOSE: u32 = 0xffffff;
}

/// Font sizes in pixels. Body text is 13px, with 11px for metadata.
pub mod typography {
    pub const CAPTION: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 16.0;
    pub const HEADING: f32 = 20.0;
}

/// Spacing on a 4px grid.
pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
}

/// Corner radii in pixels.
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
}

/// Shadow geometry for the dialog elevation.
pub mod shadow {
    pub const OFFSET_Y: f32 = 8.0;
    pub const BLUR: f32 = 24.0;
}
