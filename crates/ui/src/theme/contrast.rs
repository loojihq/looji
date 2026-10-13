//! WCAG 2.x contrast. Every foreground and background pair the UI uses is checked, in both modes.
//!
//! Text needs 4.5:1. Control outlines and the active field outline need 3:1. Disabled text is
//! exempt from WCAG but must still clear 3:1 so it stays readable.

use gpui::Hsla;

use super::{Mode, Palette};

const TEXT_MIN: f32 = 4.5;
const NON_TEXT_MIN: f32 = 3.0;

/// WCAG relative luminance of a colour, with alpha ignored.
pub fn relative_luminance(color: Hsla) -> f32 {
    let rgba = color.to_rgb();
    let linear = |channel: f32| {
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
}

/// WCAG contrast ratio, from 1 to 21, between two opaque colours.
pub fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Every pair the components use, with the minimum ratio it must meet.
///
/// Names describe where the pair appears. Add a pair here when a component starts using one.
fn pairs(p: &Palette) -> Vec<(String, Hsla, Hsla, f32)> {
    let surfaces = [
        ("surface", p.surface),
        ("surface_raised", p.surface_raised),
        ("surface_hover", p.surface_hover),
        ("surface_selected", p.surface_selected),
    ];

    let mut pairs = Vec::new();
    for (name, bg) in surfaces {
        pairs.push((
            format!("text_primary on {name}"),
            p.text_primary,
            bg,
            TEXT_MIN,
        ));
        pairs.push((
            format!("text_secondary on {name}"),
            p.text_secondary,
            bg,
            TEXT_MIN,
        ));
        // Danger text and the accent outline only ever appear on these two surfaces.
        if matches!(name, "surface" | "surface_raised") {
            pairs.push((format!("danger on {name}"), p.danger, bg, TEXT_MIN));
        }
        pairs.push((
            format!("border_control on {name}"),
            p.border_control,
            bg,
            NON_TEXT_MIN,
        ));
        pairs.push((format!("accent on {name}"), p.accent, bg, NON_TEXT_MIN));
        pairs.push((
            format!("text_disabled on {name}"),
            p.text_disabled,
            bg,
            NON_TEXT_MIN,
        ));
    }
    pairs.push((
        "text_primary on surface_active".into(),
        p.text_primary,
        p.surface_active,
        TEXT_MIN,
    ));
    pairs.push((
        "text_primary on title_bar".into(),
        p.text_primary,
        p.title_bar,
        TEXT_MIN,
    ));
    pairs.push((
        "text_primary on title_bar_inactive".into(),
        p.text_primary,
        p.title_bar_inactive,
        TEXT_MIN,
    ));
    pairs.push((
        "on_danger on danger".into(),
        p.on_danger,
        p.danger,
        TEXT_MIN,
    ));
    pairs.push((
        "on_accent on accent".into(),
        p.on_accent,
        p.accent,
        TEXT_MIN,
    ));
    for (index, avatar) in p.avatars.iter().enumerate() {
        pairs.push((
            format!("avatar {index} text on background"),
            avatar.text,
            avatar.background,
            TEXT_MIN,
        ));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_matches_the_wcag_definition_at_the_extremes() {
        assert!((relative_luminance(gpui::white()) - 1.0).abs() < 1e-3);
        assert!(relative_luminance(gpui::black()).abs() < 1e-6);
    }

    #[test]
    fn black_on_white_is_the_maximum_ratio() {
        assert!((contrast_ratio(gpui::black(), gpui::white()) - 21.0).abs() < 0.01);
    }

    #[test]
    fn every_pair_the_ui_uses_meets_its_minimum_in_both_modes() {
        for mode in [Mode::Light, Mode::Dark] {
            let palette = Palette::for_mode(mode);
            let mut failures = Vec::new();
            for (name, fg, bg, minimum) in pairs(&palette) {
                let ratio = contrast_ratio(fg, bg);
                if ratio < minimum {
                    failures.push(format!("{name}: {ratio:.2}:1, needs {minimum}:1"));
                }
            }
            assert!(
                failures.is_empty(),
                "{mode:?} mode fails:\n{}",
                failures.join("\n")
            );
        }
    }

    #[test]
    fn the_check_covers_the_pairs_the_components_use() {
        let palette = Palette::for_mode(Mode::Dark);
        let names: Vec<String> = pairs(&palette).into_iter().map(|(name, ..)| name).collect();
        for expected in [
            "text_primary on surface",
            "text_secondary on surface_selected",
            "danger on surface_raised",
            "border_control on surface_raised",
            "accent on surface",
            "on_danger on danger",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing pair {expected}"
            );
        }
    }
}
