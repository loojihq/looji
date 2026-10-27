//! Wireframes as data. Like a diagram, a wireframe is never drawn by the AI: it writes a tree of boxes and labels, this
//! module checks it, and the app draws the tree natively with the theme's colours. A wireframe sits in a document's
//! "diagrams" list with `"type": "wireframe"`, so it is stored, versioned and revised with the rest of the document.

use serde_json::Value;

/// The most elements a wireframe may have, counting containers.
pub const MAX_ELEMENTS: usize = 60;
/// How deep containers may nest.
pub const MAX_DEPTH: usize = 6;
/// The longest label kept, in characters.
const MAX_LABEL: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    /// Children stacked top to bottom.
    Column,
    /// Children side by side.
    Row,
    /// A bordered column, for a panel or a card.
    Card,
    Heading,
    Text,
    Button,
    Input,
    /// A picture placeholder.
    Image,
    Divider,
}

impl ElementKind {
    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "column" | "stack" | "screen" => Some(Self::Column),
            "row" => Some(Self::Row),
            "card" | "panel" => Some(Self::Card),
            "heading" | "title" => Some(Self::Heading),
            "text" | "label" => Some(Self::Text),
            "button" => Some(Self::Button),
            "input" | "field" => Some(Self::Input),
            "image" | "picture" => Some(Self::Image),
            "divider" => Some(Self::Divider),
            _ => None,
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, Self::Column | Self::Row | Self::Card)
    }

    fn word(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::Row => "row",
            Self::Card => "card",
            Self::Heading => "heading",
            Self::Text => "text",
            Self::Button => "button",
            Self::Input => "input",
            Self::Image => "image",
            Self::Divider => "divider",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub kind: ElementKind,
    /// The words shown. Empty for a container or a divider.
    pub label: String,
    pub children: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wireframe {
    pub title: String,
    pub root: Element,
}

/// Whether a diagram's JSON says it is a wireframe.
pub fn is_wireframe(json: &Value) -> bool {
    json.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("wireframe"))
}

fn label_of(json: &Value) -> String {
    let text = json
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    text.chars().take(MAX_LABEL).collect()
}

fn element(
    json: &Value,
    depth: usize,
    count: &mut usize,
    problems: &mut Vec<String>,
) -> Option<Element> {
    *count += 1;
    if *count > MAX_ELEMENTS {
        if *count == MAX_ELEMENTS + 1 {
            problems.push(format!(
                "A wireframe may have at most {MAX_ELEMENTS} elements."
            ));
        }
        return None;
    }
    let Some(kind) = json
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ElementKind::parse)
    else {
        problems.push(
            "Every element needs a \"kind\": column, row, card, heading, text, button, input, image, or divider."
                .to_string(),
        );
        return None;
    };
    let mut children = Vec::new();
    if let Some(list) = json.get("children").and_then(Value::as_array) {
        if !kind.is_container() && !list.is_empty() {
            problems.push(format!("A {} cannot have children.", kind.word()));
        } else if depth >= MAX_DEPTH && !list.is_empty() {
            problems.push(format!("Containers may nest at most {MAX_DEPTH} deep."));
        } else {
            children = list
                .iter()
                .filter_map(|child| element(child, depth + 1, count, problems))
                .collect();
        }
    }
    Some(Element {
        kind,
        label: label_of(json),
        children,
    })
}

/// Reads a wireframe from the JSON an AI wrote: `{"type": "wireframe", "title": "...", "root": {...}}`. Every problem
/// found is listed, so the reply can be fixed in one go.
pub fn parse_wireframe(json: &Value) -> Result<Wireframe, Vec<String>> {
    let mut problems = Vec::new();
    if !is_wireframe(json) {
        problems.push("\"type\" must be wireframe.".to_string());
    }
    let title = json
        .get("title")
        .and_then(Value::as_str)
        .map(|title| title.trim().chars().take(MAX_LABEL).collect::<String>())
        .unwrap_or_default();
    let mut count = 0;
    let root = match json.get("root") {
        Some(root) => element(root, 1, &mut count, &mut problems),
        None => {
            problems.push("A wireframe needs a \"root\" element.".to_string());
            None
        }
    };
    match (root, problems.is_empty()) {
        (Some(root), true) => Ok(Wireframe { title, root }),
        _ => Err(problems),
    }
}

impl Wireframe {
    /// The wireframe as indented text, so a search or a chat can find what a screen holds.
    pub fn outline(&self) -> String {
        let mut out = format!("Wireframe: {}\n", self.title);
        outline_into(&self.root, 0, &mut out);
        out
    }
}

fn outline_into(element: &Element, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    if element.label.is_empty() {
        out.push_str(&format!("{indent}- {}\n", element.kind.word()));
    } else {
        out.push_str(&format!(
            "{indent}- {}: {}\n",
            element.kind.word(),
            element.label
        ));
    }
    for child in &element.children {
        outline_into(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Value {
        json!({
            "type": "wireframe",
            "title": "Sign in",
            "root": {"kind": "column", "children": [
                {"kind": "heading", "label": "Welcome back"},
                {"kind": "card", "children": [
                    {"kind": "input", "label": "Email"},
                    {"kind": "input", "label": "Password"},
                    {"kind": "row", "children": [
                        {"kind": "button", "label": "Sign in"},
                        {"kind": "text", "label": "Forgot password?"}
                    ]}
                ]}
            ]}
        })
    }

    #[test]
    fn a_wireframe_is_read_and_its_outline_names_every_label() {
        let wireframe = parse_wireframe(&sample()).unwrap();
        assert_eq!(wireframe.title, "Sign in");
        assert_eq!(wireframe.root.children.len(), 2);
        let outline = wireframe.outline();
        assert!(outline.contains("Wireframe: Sign in"), "{outline}");
        assert!(outline.contains("  - heading: Welcome back"));
        assert!(outline.contains("      - button: Sign in"));
    }

    #[test]
    fn problems_are_all_listed_not_just_the_first() {
        let bad = json!({"type": "wireframe", "root": {"kind": "column", "children": [
            {"kind": "carousel"},
            {"kind": "button", "label": "Go", "children": [{"kind": "text"}]}
        ]}});
        let problems = parse_wireframe(&bad).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(parse_wireframe(&json!({"type": "wireframe"})).is_err());
        assert!(parse_wireframe(&json!({"type": "component", "root": {"kind": "row"}})).is_err());
    }

    #[test]
    fn size_and_depth_are_capped() {
        let many: Vec<Value> = (0..MAX_ELEMENTS)
            .map(|_| json!({"kind": "text", "label": "x"}))
            .collect();
        let big = json!({"type": "wireframe", "root": {"kind": "column", "children": many}});
        assert!(parse_wireframe(&big).is_err());

        let mut deep = json!({"kind": "text", "label": "bottom"});
        for _ in 0..MAX_DEPTH {
            deep = json!({"kind": "column", "children": [deep]});
        }
        let tall = json!({"type": "wireframe", "root": deep});
        assert!(parse_wireframe(&tall).is_err());
    }
}
