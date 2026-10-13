//! A workspace's circular avatar: its icon, or its first letter when there's no usable icon.

use std::path::Path;

use gpui::{prelude::*, *};
use looji_domain::{Workspace, WorkspaceId};
use looji_persistence::icons::icon_path;
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::Theme;

pub const AVATAR_SIZE: Pixels = px(28.);

/// Renders the icon if it's set and its file exists. If the file is missing or fails to load,
/// renders the initial instead.
pub fn render(workspace: &Workspace, icons_dir: &Path, theme: &Theme) -> impl IntoElement {
    let initial = initial_avatar(workspace.id(), workspace.name(), theme);
    let icon = workspace
        .icon_file()
        .map(|file| icon_path(icons_dir, file))
        .filter(|path| path.is_file());

    match icon {
        Some(path) => {
            let (id, name, colors) = (
                workspace.id(),
                workspace.name().to_string(),
                theme.avatar_colors,
            );
            img(path)
                .size(AVATAR_SIZE)
                .rounded_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(move || avatar_with(id, &name, &colors))
                .into_any_element()
        }
        None => initial,
    }
}

fn initial_avatar(id: WorkspaceId, name: &str, theme: &Theme) -> AnyElement {
    avatar_with(id, name, &theme.avatar_colors)
}

fn avatar_with(id: WorkspaceId, name: &str, colors: &[Hsla; 6]) -> AnyElement {
    let color = colors[palette_index(id, colors.len())];
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(AVATAR_SIZE)
        .rounded_full()
        .bg(color)
        .text_color(gpui::white())
        .text_sm()
        .child(initial(name))
        .into_any_element()
}

/// The first user-perceived character, uppercased. Splits on grapheme clusters, so emoji,
/// combining marks, and scripts with multi-code-point letters stay whole.
pub fn initial(name: &str) -> String {
    name.graphemes(true)
        .next()
        .map(|grapheme| grapheme.to_uppercase())
        .unwrap_or_else(|| "?".to_string())
}

/// A stable index derived from the workspace id, so a workspace keeps its colour across renames.
fn palette_index(id: WorkspaceId, len: usize) -> usize {
    // FNV-1a, 64-bit. Written out so the choice doesn't depend on std's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.as_uuid().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % len as u64) as usize
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: that would also import gpui's `test` macro and shadow `#[test]`.
    use looji_domain::WorkspaceId;

    use super::{initial, palette_index};

    #[test]
    fn initial_is_the_first_uppercased_letter() {
        assert_eq!(initial("work"), "W");
    }

    #[test]
    fn initial_keeps_emoji_whole() {
        assert_eq!(initial("🚀 launch"), "🚀");
    }

    #[test]
    fn initial_keeps_combining_marks_with_their_letter() {
        // "é" written as "e" plus a combining acute accent.
        assert_eq!(initial("e\u{301}cole"), "E\u{301}");
    }

    #[test]
    fn initial_handles_right_to_left_scripts() {
        assert_eq!(initial("שלום"), "ש");
    }

    #[test]
    fn palette_index_is_stable_and_in_range() {
        let id = WorkspaceId::from_uuid(uuid::Uuid::from_u128(42));
        assert_eq!(palette_index(id, 6), palette_index(id, 6));
        assert!(palette_index(id, 6) < 6);
    }

    #[test]
    fn palette_spreads_workspaces_across_colours() {
        let used: std::collections::HashSet<usize> = (1..=60_u128)
            .map(|n| palette_index(WorkspaceId::from_uuid(uuid::Uuid::from_u128(n)), 6))
            .collect();
        assert_eq!(used.len(), 6, "every palette colour should be used");
    }
}
