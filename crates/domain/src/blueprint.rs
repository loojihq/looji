//! One document of a project's blueprint. The blueprint is a set of these: an overview, the architecture, the data
//! model, interfaces, decisions, risks, and milestones. Diagrams are kept as JSON that the diagram crate reads, so the
//! domain stays free of it.

use crate::Timestamp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlueprintDoc {
    /// Which part of the blueprint this is, such as "architecture" or "decisions".
    pub kind: String,
    pub title: String,
    /// Markdown.
    pub body: String,
    /// A JSON array of diagrams, as written by the model and checked by the diagram crate.
    pub diagrams: String,
    /// Which version of the blueprint this document belongs to. Set when the blueprint is stored.
    pub version: u32,
    pub updated_at: Timestamp,
}

impl BlueprintDoc {
    pub fn new(kind: &str, title: &str, body: &str, diagrams: &str) -> Self {
        Self {
            kind: kind.trim().to_string(),
            title: title.trim().to_string(),
            body: body.trim().to_string(),
            diagrams: if diagrams.trim().is_empty() {
                "[]".to_string()
            } else {
                diagrams.to_string()
            },
            version: 0,
            updated_at: Timestamp::from_unix_millis(0),
        }
    }
}
