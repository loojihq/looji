//! Places a report's text on pages. The result is positioned text and rules, page by page, with nothing drawn yet.
//! Pages are filled top to bottom, and a block that doesn't fit on the rest of the page starts a new one. A block
//! never runs into the footer.

use looji_domain::{ReportItem, StatusSection, status_label};

use crate::design::{
    ACCENT, BODY_PT, BORDER, BOTTOM_MM, HEADING_PT, LINE_SPACING, MARGIN_MM, PAGE_HEIGHT_MM,
    PAGE_WIDTH_MM, PT_TO_MM, Rgb8, SMALL_PT, SUBHEADING_PT, TEXT_PRIMARY, TEXT_SECONDARY, TITLE_PT,
    TOP_MM, content_width_mm,
};
use crate::text::{Face, printable, width_mm, wrap};

/// What a report is for. Drafts are shown with their tasks under Backlog, since publishing puts them there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Published,
    Draft,
}

/// The whole report's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportInput {
    pub project: String,
    pub workspace: String,
    pub kind: ReportKind,
    pub description: String,
    /// Shown on the cover, as the date or time the report was made.
    pub generated_on: String,
    pub sections: Vec<StatusSection>,
}

/// One placed piece of a page. Positions are millimetres from the top-left corner of the page, and `baseline` is
/// the line the text sits on.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Text {
        x: f32,
        baseline: f32,
        text: String,
        size_pt: f32,
        face: Face,
        colour: Rgb8,
    },
    Rule {
        x_start: f32,
        x_end: f32,
        y: f32,
        thickness_pt: f32,
        colour: Rgb8,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub ops: Vec<Op>,
}

/// The layout cursor: the pages filled so far, and how far down the current one the next block starts.
struct Cursor {
    pages: Vec<Page>,
    y: f32,
}

impl Cursor {
    fn new() -> Self {
        Self {
            pages: vec![Page::default()],
            y: TOP_MM,
        }
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("there is always a page")
    }

    fn new_page(&mut self) {
        self.pages.push(Page::default());
        self.y = TOP_MM;
    }

    /// Starts a new page unless `height` mm fit below the cursor.
    fn reserve(&mut self, height: f32) {
        if self.y + height > PAGE_HEIGHT_MM - BOTTOM_MM {
            self.new_page();
        }
    }

    /// Places a block of wrapped text. Long blocks are broken across pages line by line.
    fn paragraph(&mut self, text: &str, size_pt: f32, face: Face, colour: Rgb8, indent_mm: f32) {
        let line_height = line_height_mm(size_pt);
        let width = content_width_mm() - indent_mm;
        for line in wrap(&printable(text), size_pt, face, width) {
            self.reserve(line_height);
            let baseline = self.y + line_height * 0.8;
            self.page().ops.push(Op::Text {
                x: MARGIN_MM + indent_mm,
                baseline,
                text: line,
                size_pt,
                face,
                colour,
            });
            self.y += line_height;
        }
    }

    /// One line with a label on the left and a value right-aligned to the margin, on the same baseline.
    fn row(&mut self, label: &str, value: &str, size_pt: f32, face: Face, colour: Rgb8) {
        let line_height = line_height_mm(size_pt);
        self.reserve(line_height);
        let baseline = self.y + line_height * 0.8;
        let value = printable(value);
        let value_x = PAGE_WIDTH_MM - MARGIN_MM - width_mm(&value, size_pt, face);
        let page = self.page();
        page.ops.push(Op::Text {
            x: MARGIN_MM,
            baseline,
            text: printable(label),
            size_pt,
            face,
            colour,
        });
        page.ops.push(Op::Text {
            x: value_x,
            baseline,
            text: value,
            size_pt,
            face,
            colour,
        });
        self.y += line_height;
    }

    fn gap(&mut self, mm: f32) {
        self.y += mm;
    }

    fn rule(&mut self, colour: Rgb8, thickness_pt: f32) {
        self.reserve(2.0);
        let y = self.y;
        self.page().ops.push(Op::Rule {
            x_start: MARGIN_MM,
            x_end: PAGE_WIDTH_MM - MARGIN_MM,
            y,
            thickness_pt,
            colour,
        });
        self.y += 2.0;
    }
}

pub fn line_height_mm(size_pt: f32) -> f32 {
    size_pt * LINE_SPACING * PT_TO_MM
}

/// Lays out the report: a cover, a summary, then one section per status in board order. Each section's heading
/// carries its count. Every page after the cover gets a running header, and every page a number.
pub fn layout(input: &ReportInput) -> Vec<Page> {
    let mut cursor = Cursor::new();
    cover(&mut cursor, input);
    cursor.new_page();
    summary(&mut cursor, input);
    for section in &input.sections {
        section_block(&mut cursor, section);
    }
    let total = cursor.pages.len();
    let mut pages = cursor.pages;
    for (index, page) in pages.iter_mut().enumerate() {
        if index > 0 {
            add_header(page, &input.project);
        }
        add_footer(page, index + 1, total);
    }
    pages
}

fn cover(cursor: &mut Cursor, input: &ReportInput) {
    cursor.y = 70.0;
    cursor.rule(ACCENT, 3.0);
    cursor.gap(6.0);
    cursor.paragraph(
        &input.workspace,
        SUBHEADING_PT,
        Face::Regular,
        TEXT_SECONDARY,
        0.0,
    );
    cursor.gap(4.0);
    cursor.paragraph(&input.project, TITLE_PT, Face::Bold, TEXT_PRIMARY, 0.0);
    cursor.gap(6.0);
    let kind = match input.kind {
        ReportKind::Published => "Project report",
        ReportKind::Draft => "Draft plan report",
    };
    cursor.paragraph(kind, 12.0, Face::Regular, TEXT_SECONDARY, 0.0);
    cursor.gap(16.0);
    cursor.paragraph(
        &input.generated_on,
        BODY_PT,
        Face::Regular,
        TEXT_SECONDARY,
        0.0,
    );
}

fn summary(cursor: &mut Cursor, input: &ReportInput) {
    cursor.paragraph("Summary", HEADING_PT, Face::Bold, TEXT_PRIMARY, 0.0);
    cursor.gap(3.0);
    let description = if input.description.trim().is_empty() {
        "This project has no description.".to_string()
    } else {
        input.description.trim().to_string()
    };
    cursor.paragraph(&description, BODY_PT, Face::Regular, TEXT_PRIMARY, 0.0);
    cursor.gap(6.0);
    for section in &input.sections {
        cursor.row(
            status_label(section.status),
            &section.count().to_string(),
            BODY_PT,
            Face::Regular,
            TEXT_PRIMARY,
        );
    }
    let total: usize = input.sections.iter().map(StatusSection::count).sum();
    cursor.rule(BORDER, 0.5);
    cursor.row(
        "Total",
        &total.to_string(),
        BODY_PT,
        Face::Bold,
        TEXT_PRIMARY,
    );
    if input.kind == ReportKind::Draft {
        cursor.gap(4.0);
        cursor.paragraph(
            "These tasks are not on the board yet. Publishing puts them in the backlog.",
            SMALL_PT,
            Face::Italic,
            TEXT_SECONDARY,
            0.0,
        );
    }
}

fn section_block(cursor: &mut Cursor, section: &StatusSection) {
    // The heading is kept with its first line, so it never sits alone at the foot of a page.
    cursor.reserve(line_height_mm(HEADING_PT) + 12.0);
    cursor.gap(8.0);
    let heading = format!("{} ({})", section.label, section.count());
    cursor.paragraph(&heading, HEADING_PT, Face::Bold, TEXT_PRIMARY, 0.0);
    cursor.gap(2.0);
    cursor.rule(BORDER, 0.5);
    cursor.gap(2.0);

    if section.items.is_empty() {
        cursor.paragraph(
            "No tasks with this status.",
            BODY_PT,
            Face::Italic,
            TEXT_SECONDARY,
            0.0,
        );
        return;
    }
    for item in &section.items {
        task_block(cursor, item);
    }
}

fn task_block(cursor: &mut Cursor, item: &ReportItem) {
    cursor.gap(3.0);
    cursor.paragraph(&item.title, SUBHEADING_PT, Face::Bold, TEXT_PRIMARY, 0.0);
    let mut meta = format!("Priority: {}", item.priority);
    if let Some(assignee) = &item.assignee {
        meta.push_str(&format!("  ·  Assignee: {assignee}"));
    }
    if let Some(due) = &item.due {
        meta.push_str(&format!("  ·  Due: {due}"));
    }
    cursor.paragraph(&meta, SMALL_PT, Face::Regular, TEXT_SECONDARY, 0.0);
    if !item.description.trim().is_empty() {
        cursor.gap(1.0);
        cursor.paragraph(
            item.description.trim(),
            BODY_PT,
            Face::Regular,
            TEXT_PRIMARY,
            4.0,
        );
    }
    cursor.gap(2.0);
}

fn add_header(page: &mut Page, project: &str) {
    page.ops.push(Op::Text {
        x: MARGIN_MM,
        baseline: 14.0,
        text: printable(project),
        size_pt: SMALL_PT,
        face: Face::Regular,
        colour: TEXT_SECONDARY,
    });
    page.ops.push(Op::Rule {
        x_start: MARGIN_MM,
        x_end: PAGE_WIDTH_MM - MARGIN_MM,
        y: 17.0,
        thickness_pt: 0.5,
        colour: BORDER,
    });
}

fn add_footer(page: &mut Page, number: usize, total: usize) {
    let label = format!("Page {number} of {total}");
    let width = width_mm(&label, SMALL_PT, Face::Regular);
    page.ops.push(Op::Text {
        x: PAGE_WIDTH_MM - MARGIN_MM - width,
        baseline: PAGE_HEIGHT_MM - 14.0,
        text: label,
        size_pt: SMALL_PT,
        face: Face::Regular,
        colour: TEXT_SECONDARY,
    });
}
