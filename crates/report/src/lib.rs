//! Lays out a project report and writes it as a PDF: a cover, a summary with the count for each status, then the
//! tasks grouped by status in board order. Every task appears once. Each page after the cover has a running header,
//! and every page a number.
//!
//! Layout is separate from drawing, so the layout can be tested without reading a PDF back. Nothing here touches the
//! filesystem. The caller writes the bytes.

pub mod design;
mod export;
mod layout;
mod render;
mod text;

use thiserror::Error;

pub use export::{ExportError, write_report};
pub use layout::{Op, Page, ReportInput, ReportKind, layout};
pub use text::{Face, width_mm, wrap};

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("the PDF could not be written: {0}")]
    Pdf(String),
}

/// The report as PDF bytes.
pub fn render(input: &ReportInput) -> Result<Vec<u8>, ReportError> {
    let pages = layout(input);
    let title = format!("{} report", input.project);
    render::draw_document(&title, &pages).map_err(|err| ReportError::Pdf(err.to_string()))
}
