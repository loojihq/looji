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

/// How wide the exported picture is, and the space around and between things, in pixels.
const PAGE_WIDTH: f32 = 560.0;
const MARGIN: f32 = 16.0;
const GAP: f32 = 8.0;
const CARD_PADDING: f32 = 10.0;

impl Wireframe {
    /// The wireframe as an SVG image, laid out the way the app draws it: rows share their width and columns stack.
    pub fn to_svg(&self) -> String {
        let width = PAGE_WIDTH - 2.0 * MARGIN;
        let mut body = String::new();
        let height = place(&self.root, MARGIN, MARGIN, width, Some(&mut body));
        let total = height + 2.0 * MARGIN;
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {PAGE_WIDTH:.0} {total:.0}\" width=\"{PAGE_WIDTH:.0}\" height=\"{total:.0}\" font-family=\"sans-serif\" font-size=\"13\">
<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>
{body}</svg>
"
        )
    }
}

fn text_at(
    x: f32,
    y: f32,
    size: f32,
    weight: &str,
    fill: &str,
    anchor: &str,
    text: &str,
) -> String {
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{fill}\" text-anchor=\"{anchor}\">{}</text>
",
        crate::export::escape(text)
    )
}

fn rect(x: f32, y: f32, w: f32, h: f32, stroke: &str, fill: &str) -> String {
    format!(
        "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"6\" fill=\"{fill}\" stroke=\"{stroke}\"/>
"
    )
}

/// Where an element goes: its height when laid out `w` wide at (`x`, `y`), and, when `out` is given, its drawing.
fn place(element: &Element, x: f32, y: f32, w: f32, mut out: Option<&mut String>) -> f32 {
    let mut draw = |piece: String| {
        if let Some(out) = out.as_deref_mut() {
            out.push_str(&piece);
        }
    };
    match element.kind {
        ElementKind::Column | ElementKind::Card => {
            let card = element.kind == ElementKind::Card;
            let pad = if card { CARD_PADDING } else { 0.0 };
            // A card's frame is drawn first so its children sit on top, which needs its height before they are drawn.
            let inner = element
                .children
                .iter()
                .map(|child| place(child, 0.0, 0.0, w - 2.0 * pad, None))
                .collect::<Vec<_>>();
            let gaps = GAP * inner.len().saturating_sub(1) as f32;
            let height = inner.iter().sum::<f32>() + gaps + 2.0 * pad;
            if card {
                draw(rect(x, y, w, height, "#b8b8c0", "#f6f6f8"));
            }
            let mut top = y + pad;
            for (child, child_height) in element.children.iter().zip(&inner) {
                if let Some(out) = out.as_deref_mut() {
                    place(child, x + pad, top, w - 2.0 * pad, Some(out));
                }
                top += child_height + GAP;
            }
            height
        }
        ElementKind::Row => {
            let count = element.children.len().max(1) as f32;
            let each = (w - GAP * (count - 1.0)) / count;
            let height = element
                .children
                .iter()
                .map(|child| place(child, 0.0, 0.0, each, None))
                .fold(0.0, f32::max);
            if let Some(out) = out {
                for (index, child) in element.children.iter().enumerate() {
                    place(child, x + index as f32 * (each + GAP), y, each, Some(out));
                }
            }
            height
        }
        ElementKind::Heading => {
            draw(text_at(
                x,
                y + 20.0,
                18.0,
                "600",
                "#202024",
                "start",
                &element.label,
            ));
            28.0
        }
        ElementKind::Text => {
            draw(text_at(
                x,
                y + 14.0,
                13.0,
                "400",
                "#5c5c66",
                "start",
                &element.label,
            ));
            20.0
        }
        ElementKind::Button => {
            draw(rect(x, y, w, 34.0, "#e8741a", "#ffffff"));
            draw(text_at(
                x + w / 2.0,
                y + 22.0,
                13.0,
                "500",
                "#e8741a",
                "middle",
                &element.label,
            ));
            34.0
        }
        ElementKind::Input => {
            draw(rect(x, y, w, 34.0, "#b8b8c0", "#ffffff"));
            draw(text_at(
                x + 10.0,
                y + 22.0,
                13.0,
                "400",
                "#8a8a94",
                "start",
                &element.label,
            ));
            34.0
        }
        ElementKind::Image => {
            draw(rect(x, y, w, 96.0, "#b8b8c0", "#ececf0"));
            let label = if element.label.is_empty() {
                "image"
            } else {
                &element.label
            };
            draw(text_at(
                x + w / 2.0,
                y + 52.0,
                12.0,
                "400",
                "#5c5c66",
                "middle",
                label,
            ));
            96.0
        }
        ElementKind::Divider => {
            draw(format!(
                "<line x1=\"{x:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#d4d4da\"/>
",
                x + w
            ));
            1.0
        }
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
    fn the_svg_holds_every_label_escaped_and_is_a_real_picture_as_a_png() {
        let mut json = sample();
        json["root"]["children"][0]["label"] = json!("Fish & chips <now>");
        let wireframe = parse_wireframe(&json).unwrap();
        let svg = wireframe.to_svg();
        assert!(svg.starts_with("<svg "));
        for label in ["Email", "Password", "Sign in", "Forgot password?"] {
            assert!(svg.contains(&format!(">{label}<")), "{label} missing");
        }
        assert!(svg.contains("Fish &amp; chips &lt;now&gt;"));
        assert!(svg.trim_end().ends_with("</svg>"));
        let png = crate::export::svg_to_png(&svg, 1.0).unwrap();
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
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
