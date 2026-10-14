//! The task card: one task on the board. Display only. Drag, selection, and editing wrap it later.
//!
//! The card takes a [`CardView`] rather than a domain `Task`, because priority, due dates, and
//! assignee names aren't in the domain yet. The board fills in what exists and leaves the rest
//! `None`. Every field the card can show is already wired, so each one lights up once it's stored.

use gpui::{prelude::*, *};
use looji_domain::{TaskId, Timestamp};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::{ActiveTheme, Theme};

/// Compact is the board default, with many cards per screen. Detailed gives each field more room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))] // Detailed is used by the debug gallery until boards choose density.
pub enum CardDensity {
    Compact,
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))] // Set by the priority field, which isn't stored yet.
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    /// A glyph and a word, so the marker doesn't depend on colour alone.
    fn glyph(self) -> &'static str {
        match self {
            Priority::High => "!!!",
            Priority::Medium => "!!",
            Priority::Low => "!",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Priority::High => "High priority",
            Priority::Medium => "Medium priority",
            Priority::Low => "Low priority",
        }
    }
}

/// A calendar date with no time or zone. Dates come in from the caller, so the card never reads
/// the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CalendarDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl CalendarDate {
    /// `None` if the month or day doesn't exist, including February 29 in a non-leap year.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Today's date in UTC, from the clock. Called once by the board and passed down.
    pub fn utc_today() -> Self {
        let days = Timestamp::now().unix_millis().div_euclid(86_400_000);
        let (year, month, day) = civil_from_days(days);
        Self { year, month, day }
    }
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Converts days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
pub(super) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year as i32, month, day)
}

/// Everything the card shows. Borrowed, so building a card allocates nothing for the title.
#[derive(Debug, Clone)]
pub struct CardView<'a> {
    pub id: TaskId,
    pub title: &'a str,
    pub priority: Option<Priority>,
    /// The assignee's full name. Initials are derived from it.
    pub assignee: Option<&'a str>,
    pub estimate_minutes: Option<u32>,
    pub due: Option<CalendarDate>,
}

/// Builds a card for `view`. A function rather than a `RenderOnce` type, because `IntoElement`
/// needs `'static` data and the view borrows the task.
pub fn task_card(
    view: CardView<'_>,
    density: CardDensity,
    today: CalendarDate,
    theme: &Theme,
) -> impl IntoElement + use<> {
    card_layout(&view, density, today, theme)
}

/// The one layout both densities share. Density changes spacing and line count, nothing else.
fn card_layout(
    view: &CardView<'_>,
    density: CardDensity,
    today: CalendarDate,
    theme: &Theme,
) -> impl IntoElement + use<> {
    let (padding, gap, avatar) = match density {
        CardDensity::Compact => (theme.spacing.xs, theme.spacing.xs, px(20.)),
        CardDensity::Detailed => (theme.spacing.sm, theme.spacing.sm, px(24.)),
    };

    div()
        .id(view.id.as_uuid())
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .gap(gap)
        .px(theme.spacing.sm)
        .py(padding)
        .rounded(theme.radius.sm)
        .bg(theme.palette.surface)
        .border_1()
        .border_color(theme.palette.border)
        .text_size(theme.typography.body)
        .child(title_row(view, density, theme))
        .child(footer(view, today, avatar, theme))
}

fn title_row(view: &CardView<'_>, density: CardDensity, theme: &Theme) -> impl IntoElement + use<> {
    // Both densities clamp the same element. Compact allows one line and detailed allows two.
    let lines = match density {
        CardDensity::Compact => 1,
        CardDensity::Detailed => 2,
    };
    let title = div().flex_1().min_w_0().overflow_hidden().child(TitleText {
        text: view.title.to_string().into(),
        lines,
    });
    // The marker sits beside the title in a flex row, which gives the title a definite width.
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(theme.spacing.xs)
        .child(priority_marker(view.priority, theme))
        .child(title)
}

/// The card title, cut to the width it's given. gpui's own ellipsis panics on some text (an emoji
/// at the cut), so the cut happens here instead: the text is measured at paint width and split by
/// [`cut_lines`].
struct TitleText {
    text: SharedString,
    lines: usize,
}

impl IntoElement for TitleText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TitleText {
    /// The line height, needed again when painting.
    type RequestLayoutState = Pixels;
    type PrepaintState = Vec<ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Pixels) {
        let line_height = window.text_style().line_height_in_pixels(window.rem_size());
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * self.lines as f32).into();
        (window.request_layout(style, [], cx), line_height)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _line_height: &mut Pixels,
        window: &mut Window,
        _cx: &mut App,
    ) -> Vec<ShapedLine> {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let measure = |text: &str| {
            window
                .text_system()
                .layout_line(text, font_size, &[style.to_run(text.len())], None)
                .width
        };
        cut_lines(&self.text, bounds.size.width, self.lines, measure)
            .into_iter()
            .map(|line| {
                let runs = [style.to_run(line.len())];
                window
                    .text_system()
                    .shape_line(line.into(), font_size, &runs, None)
            })
            .collect()
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line_height: &mut Pixels,
        lines: &mut Vec<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (index, line) in lines.iter().enumerate() {
            let origin = bounds.origin + point(px(0.), *line_height * index as f32);
            let _ = line.paint(origin, *line_height, window, cx);
        }
    }
}

/// Splits `text` into at most `max_lines` lines that each fit `width`. Lines break at a space
/// when there is one, otherwise between graphemes. If text is left over, the last line ends in
/// "…". `measure` gives the width of a string, so this stays pure and testable.
fn cut_lines(
    text: &str,
    width: Pixels,
    max_lines: usize,
    measure: impl Fn(&str) -> Pixels,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        if measure(rest) <= width {
            lines.push(rest.to_string());
            break;
        }
        if lines.len() + 1 == max_lines {
            lines.push(fit_with_ellipsis(rest, width, &measure));
            break;
        }
        let end = fitting_end(rest, width, &measure);
        let break_at = rest[..end]
            .rfind(char::is_whitespace)
            .filter(|&index| index > 0)
            .unwrap_or(end);
        lines.push(rest[..break_at].trim_end().to_string());
        rest = rest[break_at..].trim_start();
    }
    lines
}

/// The byte offset where the longest grapheme prefix of `text` that fits `width` ends. Always at
/// least one grapheme, so a grapheme wider than the line still makes progress.
fn fitting_end(text: &str, width: Pixels, measure: &impl Fn(&str) -> Pixels) -> usize {
    let ends = grapheme_ends(text);
    let fitting = ends.partition_point(|&end| measure(&text[..end]) <= width);
    ends[fitting.saturating_sub(1)]
}

/// The longest grapheme prefix of `text` that, followed by "…", fits `width`.
fn fit_with_ellipsis(text: &str, width: Pixels, measure: &impl Fn(&str) -> Pixels) -> String {
    let ends = grapheme_ends(text);
    let fitting = ends.partition_point(|&end| measure(&format!("{}…", &text[..end])) <= width);
    match fitting {
        0 => "…".to_string(),
        count => format!("{}…", text[..ends[count - 1]].trim_end()),
    }
}

/// The byte offset just past each grapheme in `text`.
fn grapheme_ends(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .collect()
}

/// A fixed-width slot, so titles line up whether or not a priority is set.
fn priority_marker(priority: Option<Priority>, theme: &Theme) -> impl IntoElement + use<> {
    let slot = div()
        .flex_shrink_0()
        .w(px(20.))
        .text_size(theme.typography.caption)
        .flex()
        .justify_center();
    match priority {
        Some(priority) => {
            let color = match priority {
                Priority::High => theme.palette.danger,
                Priority::Medium => theme.palette.accent,
                Priority::Low => theme.palette.text_secondary,
            };
            slot.id(("priority", priority as u64))
                .text_color(color)
                .child(priority.glyph())
                .tooltip(tooltip(priority.label()))
                .into_any_element()
        }
        None => slot.into_any_element(),
    }
}

fn footer(
    view: &CardView<'_>,
    today: CalendarDate,
    avatar: Pixels,
    theme: &Theme,
) -> impl IntoElement + use<> {
    let estimate = view
        .estimate_minutes
        .and_then(format_estimate)
        .unwrap_or_default();
    let due = view
        .due
        .map(|date| format_due(date, today))
        .unwrap_or_default();

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(theme.spacing.sm)
        .text_size(theme.typography.caption)
        .text_color(theme.palette.text_secondary)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.spacing.sm)
                .min_w_0()
                .child(div().min_w(px(36.)).child(estimate))
                .child(div().min_w(px(52.)).truncate().child(due)),
        )
        .child(assignee_badge(view.assignee, avatar, theme))
}

/// The assignee circle. With no assignee it's a neutral grey circle with "?", labelled
/// "Unassigned", rather than an error-looking mark.
fn assignee_badge(assignee: Option<&str>, size: Pixels, theme: &Theme) -> impl IntoElement + use<> {
    let (text, label, background, foreground) = match assignee {
        Some(name) => (
            initials(name).unwrap_or_else(|| "?".to_string()),
            name.to_string(),
            theme.palette.surface_selected,
            theme.palette.text_primary,
        ),
        None => (
            "?".to_string(),
            "Unassigned".to_string(),
            theme.palette.surface_active,
            theme.palette.text_secondary,
        ),
    };
    let id = ("assignee", assignee.map_or(0, |name| name.len() as u64 + 1));
    div()
        .id(id)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_full()
        .bg(background)
        .text_color(foreground)
        .text_size(theme.typography.caption)
        .child(text)
        .tooltip(tooltip(label))
}

/// A tooltip that shows one line of text.
fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| TooltipText(text.clone())).into()
}

struct TooltipText(SharedString);

impl Render for TooltipText {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div()
            .px(theme.spacing.sm)
            .py(theme.spacing.xs)
            .rounded(theme.radius.sm)
            .bg(theme.palette.surface_raised)
            .border_1()
            .border_color(theme.palette.border)
            .text_size(theme.typography.caption)
            .child(self.0.clone())
    }
}

/// Up to two initials from a name: the first character of the first and last words, uppercased.
/// `None` for an empty or blank name. Works on characters, so accented, CJK, and emoji names are
/// safe.
pub fn initials(name: &str) -> Option<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = words.first()?.graphemes(true).next()?;
    let last = words.last()?.graphemes(true).next()?;
    let mut text = first.to_uppercase();
    if words.len() > 1 {
        text.push_str(&last.to_uppercase());
    }
    Some(text)
}

/// A duration in minutes, shown in hours: `6h`, `1.5h`, `0.75h`. `None` for zero, which means no
/// estimate, so the field is hidden.
pub fn format_estimate(minutes: u32) -> Option<String> {
    if minutes == 0 {
        return None;
    }
    if minutes.is_multiple_of(60) {
        return Some(format!("{}h", minutes / 60));
    }
    let hours = f64::from(minutes) / 60.0;
    let text = format!("{hours:.2}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    Some(format!("{trimmed}h"))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `Today` for today, otherwise `Oct 20`. A year is added only when it differs from this year.
pub fn format_due(date: CalendarDate, today: CalendarDate) -> String {
    if date == today {
        return "Today".to_string();
    }
    let month = MONTHS[(date.month - 1) as usize];
    if date.year == today.year {
        format!("{month} {}", date.day)
    } else {
        format!("{month} {}, {}", date.day, date.year)
    }
}

#[cfg(debug_assertions)]
pub(super) fn gallery(theme: &Theme) -> impl IntoElement + use<> {
    // Fixtures for the visual check. Compiled only in debug builds.
    let today = CalendarDate::new(2026, 10, 13).expect("valid fixture date");
    let long_no_spaces = "x".repeat(200);
    type Case<'a> = (
        &'a str,
        Option<Priority>,
        Option<&'a str>,
        Option<u32>,
        Option<CalendarDate>,
    );
    let cases: Vec<Case<'_>> = vec![
        (
            &long_no_spaces,
            Some(Priority::High),
            Some("Dana Kim"),
            Some(6),
            CalendarDate::new(2026, 10, 13),
        ),
        (
            "Write the onboarding guide for the first week of every new workspace member",
            Some(Priority::Medium),
            Some("Ana María López"),
            Some(90),
            CalendarDate::new(2026, 10, 20),
        ),
        (
            "田中太郎さんとのミーティング資料を準備する",
            Some(Priority::Low),
            Some("田中太郎"),
            Some(45),
            CalendarDate::new(2027, 1, 5),
        ),
        ("Fix login bug", None, None, None, None),
        (
            "Unassigned task with every field",
            Some(Priority::High),
            None,
            Some(120),
            CalendarDate::new(2026, 10, 13),
        ),
        (
            "🚀 Ship the release",
            Some(Priority::Low),
            Some("Mo"),
            Some(30),
            None,
        ),
    ];

    let column = |density: CardDensity| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(theme.spacing.sm)
            .p(theme.spacing.sm)
            .children(cases.iter().enumerate().map(
                |(index, (title, priority, assignee, estimate, due))| {
                    task_card(
                        CardView {
                            id: TaskId::from_uuid(uuid::Uuid::from_u128(index as u128 + 1)),
                            title,
                            priority: *priority,
                            assignee: *assignee,
                            estimate_minutes: *estimate,
                            due: *due,
                        },
                        density,
                        today,
                        theme,
                    )
                },
            ))
    };

    div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h_0()
        .gap(theme.spacing.md)
        .p(theme.spacing.lg)
        .child(column(CardDensity::Compact))
        .child(column(CardDensity::Detailed))
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: gpui's `test` macro would shadow `#[test]`.
    use gpui::px;

    use super::{CalendarDate, civil_from_days, cut_lines, format_due, format_estimate, initials};

    /// Ten pixels per character, so widths are easy to reason about.
    fn measure(text: &str) -> gpui::Pixels {
        px(text.chars().count() as f32 * 10.)
    }

    #[test]
    fn text_that_fits_is_not_cut() {
        assert_eq!(cut_lines("Fix login", px(100.), 1, measure), ["Fix login"]);
    }

    #[test]
    fn one_line_is_cut_with_an_ellipsis() {
        assert_eq!(cut_lines("abcdefghij", px(50.), 1, measure), ["abcd…"]);
    }

    #[test]
    fn two_lines_break_at_a_space() {
        assert_eq!(
            cut_lines("one two three", px(70.), 2, measure),
            ["one", "two th…"]
        );
    }

    #[test]
    fn a_word_too_long_for_a_line_is_cut_between_characters() {
        assert_eq!(
            cut_lines("abcdefghijklmnop", px(70.), 2, measure),
            ["abcdefg", "hijklm…"]
        );
    }

    #[test]
    fn a_line_too_narrow_for_the_ellipsis_shows_only_the_ellipsis() {
        assert_eq!(cut_lines("abc", px(5.), 1, measure), ["…"]);
    }

    #[test]
    fn emoji_are_never_split() {
        let lines = cut_lines("🚀 Ship the release", px(60.), 1, measure);
        assert_eq!(lines, ["🚀 Shi…"]);
    }

    #[test]
    fn leading_spaces_are_dropped() {
        assert_eq!(cut_lines("   Fix", px(100.), 1, measure), ["Fix"]);
    }

    #[test]
    fn initials_of_one_word_is_one_letter() {
        assert_eq!(initials("dana"), Some("D".to_string()));
    }

    #[test]
    fn initials_are_first_and_last_word() {
        assert_eq!(initials("Ana María López"), Some("AL".to_string()));
    }

    #[test]
    fn initials_ignore_leading_and_trailing_spaces() {
        assert_eq!(initials("   dana   kim  "), Some("DK".to_string()));
    }

    #[test]
    fn empty_and_blank_names_have_no_initials() {
        assert_eq!(initials(""), None);
        assert_eq!(initials("   \t "), None);
    }

    #[test]
    fn accented_initials_keep_the_whole_letter() {
        assert_eq!(initials("Émile Zola"), Some("ÉZ".to_string()));
    }

    #[test]
    fn cjk_names_do_not_panic_and_give_one_character() {
        assert_eq!(initials("田中太郎"), Some("田".to_string()));
        assert_eq!(initials("田中 太郎"), Some("田太".to_string()));
    }

    #[test]
    fn emoji_are_kept_whole() {
        assert_eq!(initials("🚀 Launch"), Some("🚀L".to_string()));
    }

    #[test]
    fn zero_estimates_are_hidden() {
        assert_eq!(format_estimate(0), None);
    }

    #[test]
    fn whole_hours_have_no_decimals() {
        assert_eq!(format_estimate(60).as_deref(), Some("1h"));
        assert_eq!(format_estimate(360).as_deref(), Some("6h"));
    }

    #[test]
    fn fractional_hours_drop_trailing_zeros() {
        assert_eq!(format_estimate(90).as_deref(), Some("1.5h"));
        assert_eq!(format_estimate(30).as_deref(), Some("0.5h"));
        assert_eq!(format_estimate(45).as_deref(), Some("0.75h"));
    }

    fn date(year: i32, month: u32, day: u32) -> CalendarDate {
        CalendarDate::new(year, month, day).unwrap()
    }

    #[test]
    fn today_is_shown_as_today() {
        assert_eq!(format_due(date(2026, 10, 13), date(2026, 10, 13)), "Today");
    }

    #[test]
    fn same_year_dates_omit_the_year() {
        assert_eq!(format_due(date(2026, 10, 20), date(2026, 10, 13)), "Oct 20");
    }

    #[test]
    fn other_years_include_the_year() {
        assert_eq!(
            format_due(date(2027, 1, 5), date(2026, 10, 13)),
            "Jan 5, 2027"
        );
    }

    #[test]
    fn impossible_dates_are_rejected() {
        assert_eq!(CalendarDate::new(2026, 2, 29), None);
        assert_eq!(CalendarDate::new(2028, 2, 29), Some(date(2028, 2, 29)));
        assert_eq!(CalendarDate::new(2026, 13, 1), None);
        assert_eq!(CalendarDate::new(2026, 4, 31), None);
    }

    #[test]
    fn civil_dates_convert_from_days_since_the_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_383), (2025, 10, 22));
    }
}
