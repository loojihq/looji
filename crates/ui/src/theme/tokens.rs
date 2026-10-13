//! Raw token values. This is the only file that spells out colours, sizes, and radii for the UI.
//!
//! Surfaces are a neutral blue-grey ramp, saturation about 0.02. Each mode's accent is the only
//! strongly saturated colour. `palette.rs` gives these values their roles, and components only
//! ever read roles. The contrast tests in `contrast.rs` check every role pair the UI uses.

/// Dark mode. Soft off-black surfaces avoid the glare of pure black.
pub mod dark {
    pub const SURFACE: u32 = 0x282c33;
    pub const SURFACE_RAISED: u32 = 0x2f343e;
    pub const SURFACE_HOVER: u32 = 0x363c46;
    pub const SURFACE_SELECTED: u32 = 0x454a56;
    pub const SURFACE_ACTIVE: u32 = 0x4b505c;
    pub const TITLE_BAR: u32 = 0x3b414d;
    pub const TITLE_BAR_INACTIVE: u32 = 0x2e343e;
    /// Dividers and card edges. Decorative, so it's not held to the 3:1 control rule.
    pub const BORDER: u32 = 0x464b57;
    /// Input, button, and field outlines. Held to 3:1 against their surface.
    pub const BORDER_CONTROL: u32 = 0x9298a5;
    pub const TEXT_PRIMARY: u32 = 0xdce0e5;
    pub const TEXT_SECONDARY: u32 = 0xb4bac4;
    /// Exempt from WCAG, but kept readable enough to use.
    pub const TEXT_DISABLED: u32 = 0x9ca0ad;
    pub const ACCENT: u32 = 0x74ade8;
    pub const DANGER: u32 = 0xe0878c;
    pub const ON_DANGER: u32 = 0x1b1d22;
    pub const ON_ACCENT: u32 = 0x1b1d22;
    /// Overlay colour, black at this alpha. Drawn behind dialogs.
    pub const SCRIM_ALPHA: f32 = 0.5;
    /// Shadow colour for the dialog elevation, black at this alpha.
    pub const SHADOW_ALPHA: f32 = 0.35;
    /// Avatar pairs, as (background, text). Each pair must clear 4.5:1.
    pub const AVATARS: [(u32, u32); 6] = [
        (0x74ade8, ON_ACCENT),
        (0xa1c181, ON_ACCENT),
        (0xdec184, ON_ACCENT),
        (0xe0878c, ON_ACCENT),
        (0xb477cf, ON_ACCENT),
        (0x6fb3c0, ON_ACCENT),
    ];
}

/// Light mode. Off-white surfaces avoid the glare of pure white on bright displays.
pub mod light {
    pub const SURFACE: u32 = 0xf7f7f8;
    pub const SURFACE_RAISED: u32 = 0xeeeff1;
    pub const SURFACE_HOVER: u32 = 0xe4e5e9;
    pub const SURFACE_SELECTED: u32 = 0xdcdde3;
    pub const SURFACE_ACTIVE: u32 = 0xd2d4db;
    pub const TITLE_BAR: u32 = 0xe8e9ec;
    pub const TITLE_BAR_INACTIVE: u32 = 0xf1f2f4;
    pub const BORDER: u32 = 0xd6d8de;
    pub const BORDER_CONTROL: u32 = 0x6b717b;
    pub const TEXT_PRIMARY: u32 = 0x1f2328;
    pub const TEXT_SECONDARY: u32 = 0x555a63;
    pub const TEXT_DISABLED: u32 = 0x767b85;
    pub const ACCENT: u32 = 0x2f6fd6;
    pub const DANGER: u32 = 0xb42318;
    pub const ON_DANGER: u32 = 0xffffff;
    pub const ON_ACCENT: u32 = 0xffffff;
    pub const SCRIM_ALPHA: f32 = 0.35;
    pub const SHADOW_ALPHA: f32 = 0.18;
    pub const AVATARS: [(u32, u32); 6] = [
        (0x2f6fd6, ON_ACCENT),
        (0x2e7d4f, ON_ACCENT),
        (0x8a5a00, ON_ACCENT),
        (0xb42318, ON_ACCENT),
        (0x7a3fb0, ON_ACCENT),
        (0x1f7a8c, ON_ACCENT),
    ];
}

/// Font sizes in pixels. Body text is 14px, the size the sidebar and dialogs use today.
pub mod typography {
    pub const CAPTION: f32 = 12.0;
    pub const BODY: f32 = 14.0;
    pub const TITLE: f32 = 18.0;
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
