//! The report's layout and its PDF. Layout is checked on the placed text, and the PDF is checked by reading it back with
//! a separate PDF library, so a file that a standard reader can't open fails here too.

use looji_domain::{ReportItem, Status, group_by_status};
use looji_report::{
    Op, ReportInput, ReportKind,
    design::{BOTTOM_MM, MARGIN_MM, PAGE_HEIGHT_MM, PAGE_WIDTH_MM},
    layout, render, width_mm,
};

fn item(title: &str, status: Status, description: &str) -> ReportItem {
    ReportItem {
        title: title.to_string(),
        description: description.to_string(),
        status,
        priority: "medium".to_string(),
        assignee: Some("Dana".to_string()),
        due: Some("2026-10-20".to_string()),
    }
}

/// A project with tasks in four of the five statuses, and nothing In progress.
fn sample() -> ReportInput {
    let items = vec![
        item("Write the spec", Status::Done, "Agreed with the team."),
        item("Build login", Status::Done, ""),
        item(
            "Review copy",
            Status::InReview,
            "Two sections need a second pass.",
        ),
        item("Draft invoices", Status::Todo, "Start with the totals."),
        item("Plan the release", Status::Backlog, ""),
        item("Name the project", Status::Backlog, "Pick something short."),
    ];
    ReportInput {
        project: "Website relaunch".to_string(),
        workspace: "Marketing".to_string(),
        kind: ReportKind::Published,
        description: "Ship the new site before the autumn campaign.".to_string(),
        generated_on: "Generated 17 October 2026".to_string(),
        sections: group_by_status(&items),
    }
}

fn texts(pages: &[looji_report::Page]) -> Vec<String> {
    pages
        .iter()
        .flat_map(|page| page.ops.iter())
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text.clone()),
            Op::Rule { .. } => None,
        })
        .collect()
}

#[test]
fn every_task_appears_exactly_once_in_the_layout() {
    let input = sample();
    let pages = layout(&input);
    let all = texts(&pages);

    for title in [
        "Write the spec",
        "Build login",
        "Review copy",
        "Draft invoices",
        "Plan the release",
        "Name the project",
    ] {
        let count = all.iter().filter(|text| text.as_str() == title).count();
        assert_eq!(count, 1, "{title} should appear exactly once");
    }
}

#[test]
fn sections_come_in_board_order_with_their_counts() {
    let pages = layout(&sample());
    let all = texts(&pages);

    let order = [
        "Backlog (2)",
        "To do (1)",
        "In progress (0)",
        "In review (1)",
        "Done (2)",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|heading| {
            all.iter()
                .position(|text| text == heading)
                .unwrap_or_else(|| panic!("no heading {heading}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{positions:?}"
    );
}

#[test]
fn a_status_with_no_tasks_still_appears_with_a_note() {
    let all = texts(&layout(&sample()));

    assert!(all.iter().any(|text| text == "In progress (0)"));
    assert!(all.iter().any(|text| text == "No tasks with this status."));
}

#[test]
fn a_long_title_and_description_wrap_inside_the_margins_and_the_page() {
    let mut input = sample();
    let long_title = "Rebuild the entire checkout flow so that a returning customer can finish paying in one step without ".repeat(3);
    let long_description =
        "A very long description with many words in it that must wrap across several lines. "
            .repeat(40);
    input.sections = group_by_status(&[item(&long_title, Status::InProgress, &long_description)]);

    let pages = layout(&input);

    let right_edge = PAGE_WIDTH_MM - MARGIN_MM + 0.01;
    for page in &pages {
        for op in &page.ops {
            if let Op::Text {
                x,
                baseline,
                text,
                size_pt,
                face,
                ..
            } = op
            {
                let end = x + width_mm(text, *size_pt, *face);
                assert!(end <= right_edge, "text runs past the margin: {text:?}");
                // The footer sits below the body area, so only body text is held to the bottom margin.
                if *baseline < PAGE_HEIGHT_MM - 20.0 {
                    assert!(
                        *baseline <= PAGE_HEIGHT_MM - BOTTOM_MM + 0.01,
                        "body text reaches the footer: {text:?}"
                    );
                }
            }
        }
    }
    let title_lines = texts(&pages)
        .into_iter()
        .filter(|text| !text.is_empty() && long_title.contains(text.as_str()) && text.len() > 3)
        .count();
    assert!(
        title_lines > 3,
        "the long title is spread over several lines, not cut"
    );
}

#[test]
fn a_draft_says_its_tasks_go_to_the_backlog_on_publish() {
    let mut input = sample();
    input.kind = ReportKind::Draft;

    let all = texts(&layout(&input));

    assert!(
        all.iter()
            .any(|text| text.contains("Publishing puts them in the backlog"))
    );
}

#[test]
fn every_page_after_the_cover_has_a_header_and_every_page_a_number() {
    let input = sample();
    let pages = layout(&input);
    let total = pages.len();

    for (index, page) in pages.iter().enumerate() {
        let number = format!("Page {} of {total}", index + 1);
        assert!(
            page.ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if *text == number)),
            "page {} is missing {number}",
            index + 1
        );
        if index > 0 {
            assert!(
                page.ops
                    .iter()
                    .any(|op| matches!(op, Op::Text { text, .. } if text == "Website relaunch")),
                "page {} has no running header",
                index + 1
            );
        }
    }
}

#[test]
fn a_sample_project_exports_to_a_pdf_a_reader_can_open_and_read_back() {
    let input = sample();
    let bytes = render(&input).unwrap();

    assert!(
        bytes.starts_with(b"%PDF-"),
        "the file starts with a PDF header"
    );
    let document = lopdf::Document::load_mem(&bytes).expect("a standard PDF reader opens it");
    let page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
    assert_eq!(page_numbers.len(), layout(&input).len());
    let first = document.extract_text(&[1]).unwrap();
    assert!(first.contains("Website relaunch"), "{first}");
    let last = document
        .extract_text(&[*page_numbers.last().unwrap()])
        .unwrap();
    assert!(last.contains("Page"), "{last}");
}

#[test]
fn a_project_with_a_long_title_still_writes_a_valid_pdf() {
    let mut input = sample();
    input.project = "A project name that goes on for a long time ".repeat(12);
    let bytes = render(&input).unwrap();
    assert!(lopdf::Document::load_mem(&bytes).is_ok());
}

#[test]
fn an_empty_project_still_writes_a_cover_and_summary() {
    let input = ReportInput {
        project: "Empty".to_string(),
        workspace: "Work".to_string(),
        kind: ReportKind::Published,
        description: String::new(),
        generated_on: "today".to_string(),
        sections: group_by_status(&[]),
    };
    let pages = layout(&input);
    let all = texts(&pages);
    assert!(
        all.iter()
            .any(|text| text == "This project has no description.")
    );
    assert!(all.iter().any(|text| text == "Total"));
    let bytes = render(&input).unwrap();
    assert!(lopdf::Document::load_mem(&bytes).is_ok());
}
