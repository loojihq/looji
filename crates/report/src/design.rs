//! The report's colours and sizes. The colours are the app's light-mode palette, so a printed report reads as the
//! app does. Sizes are in points and millimetres, as a page needs them.

/// A colour as the three channels in 0..=255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb8(pub u8, pub u8, pub u8);

pub const TEXT_PRIMARY: Rgb8 = Rgb8(0x1f, 0x23, 0x28);
pub const TEXT_SECONDARY: Rgb8 = Rgb8(0x55, 0x5a, 0x63);
pub const BORDER: Rgb8 = Rgb8(0xd6, 0xd8, 0xde);
pub const ACCENT: Rgb8 = Rgb8(0x2f, 0x6f, 0xd6);
pub const SURFACE_RAISED: Rgb8 = Rgb8(0xee, 0xef, 0xf1);

pub const PAGE_WIDTH_MM: f32 = 210.0;
pub const PAGE_HEIGHT_MM: f32 = 297.0;
pub const MARGIN_MM: f32 = 22.0;
/// Where body text starts on a page, below the running header.
pub const TOP_MM: f32 = 30.0;
/// Where body text must stop, above the footer.
pub const BOTTOM_MM: f32 = 26.0;

pub const TITLE_PT: f32 = 26.0;
pub const HEADING_PT: f32 = 15.0;
pub const SUBHEADING_PT: f32 = 11.0;
pub const BODY_PT: f32 = 10.0;
pub const SMALL_PT: f32 = 8.5;

pub const LINE_SPACING: f32 = 1.35;
pub const PT_TO_MM: f32 = 0.3528;

pub fn content_width_mm() -> f32 {
    PAGE_WIDTH_MM - 2.0 * MARGIN_MM
}
