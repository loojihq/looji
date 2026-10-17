//! Text for the built-in PDF fonts. Those fonts only have the Windows-1252 characters, so anything else is replaced,
//! and widths are estimated, since the built-in fonts don't report them. The estimate is wider than Helvetica's real
//! widths, so a line that fits the estimate also fits the page.

use crate::design::PT_TO_MM;

/// The bold and italic faces are wider than regular, so they get a larger estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Regular,
    Bold,
    Italic,
}

/// Replaces characters the built-in fonts can't draw. Common typographic punctuation is mapped to its plain form.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            '\u{2026}' => '.',
            '\t' => ' ',
            ' '..='~' | '\u{a0}'..='\u{ff}' => c,
            _ => '?',
        })
        .collect()
}

/// The width of `text` in millimetres at `size_pt`, estimated per character.
pub fn width_mm(text: &str, size_pt: f32, face: Face) -> f32 {
    let per_char_em = match face {
        Face::Regular | Face::Italic => 0.6,
        Face::Bold => 0.66,
    };
    text.chars().count() as f32 * size_pt * per_char_em * PT_TO_MM
}

/// Breaks text into lines no wider than `max_mm`. Existing line breaks are kept. A word too long for a line is broken
/// by characters.
pub fn wrap(text: &str, size_pt: f32, face: Face, max_mm: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if width_mm(&candidate, size_pt, face) <= max_mm {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // The word on its own may still be too wide. Break it into pieces that fit.
            let mut piece = String::new();
            for c in word.chars() {
                piece.push(c);
                if width_mm(&piece, size_pt, face) > max_mm && piece.chars().count() > 1 {
                    piece.pop();
                    lines.push(std::mem::take(&mut piece));
                    piece.push(c);
                }
            }
            line = piece;
        }
        if !line.is_empty() || paragraph.trim().is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use crate::design::content_width_mm;
    use crate::text::{Face, printable, width_mm, wrap};

    #[test]
    fn characters_outside_the_built_in_fonts_are_replaced() {
        assert_eq!(
            printable("Ship \u{2018}v2\u{2019} \u{2014} done \u{1f680}"),
            "Ship 'v2' - done ?"
        );
        assert_eq!(printable("café"), "café");
    }

    #[test]
    fn every_wrapped_line_fits_the_width() {
        let long = "word ".repeat(200);
        for line in wrap(&long, 10.0, Face::Regular, content_width_mm()) {
            assert!(
                width_mm(&line, 10.0, Face::Regular) <= content_width_mm() + 0.001,
                "{line}"
            );
        }
    }

    #[test]
    fn a_word_longer_than_a_line_is_broken_and_nothing_is_lost() {
        let word = "x".repeat(500);
        let lines = wrap(&word, 26.0, Face::Bold, content_width_mm());
        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|line| width_mm(line, 26.0, Face::Bold) <= content_width_mm() + 0.001)
        );
        assert_eq!(lines.concat(), word);
    }

    #[test]
    fn existing_line_breaks_are_kept() {
        assert_eq!(wrap("one\ntwo", 10.0, Face::Regular, 100.0), ["one", "two"]);
    }
}
