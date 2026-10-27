//! The graph an AI writes, and the checks it must pass. Every rule is here, so a reply that passes can be laid out and
//! drawn without further surprises: ids are unique, edges join nodes that exist, groups exist, and the size is capped.

use std::collections::HashSet;

use serde_json::Value;

/// The most nodes a diagram may have. Past this a diagram stops being readable, and the model should split it.
pub const MAX_NODES: usize = 40;
/// The longest label shown, in characters.
pub const MAX_LABEL: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphKind {
    /// Parts of a system and how they depend on each other.
    Component,
    /// Steps and the order they happen in.
    Flow,
    /// Data entities and how they relate.
    Entity,
    /// Where things run.
    Deployment,
}

impl GraphKind {
    pub fn stored(self) -> &'static str {
        match self {
            GraphKind::Component => "component",
            GraphKind::Flow => "flow",
            GraphKind::Entity => "entity",
            GraphKind::Deployment => "deployment",
        }
    }

    pub fn from_stored(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "component" => Some(GraphKind::Component),
            "flow" | "sequence" => Some(GraphKind::Flow),
            "entity" | "er" | "entity_relationship" => Some(GraphKind::Entity),
            "deployment" => Some(GraphKind::Deployment),
            _ => None,
        }
    }
}

/// What a node points at, so a click can open it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A file in the repository.
    File(String),
    /// A section of the blueprint, by its title.
    Section(String),
    /// An issue, by its title or id.
    Issue(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub label: String,
    /// What sort of thing it is, such as "service" or "database". Free text, shown small under the label.
    pub kind: Option<String>,
    pub group: Option<String>,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeStyle {
    #[default]
    Solid,
    /// An optional or asynchronous link.
    Dashed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub style: EdgeStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub kind: GraphKind,
    pub title: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub groups: Vec<Group>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    let found = value.get(key)?.as_str()?.trim();
    (!found.is_empty()).then(|| found.chars().take(MAX_LABEL * 2).collect())
}

fn short(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL {
        return label.to_string();
    }
    let mut cut: String = label.chars().take(MAX_LABEL - 1).collect();
    cut.push('\u{2026}');
    cut
}

fn link_of(value: &Value) -> Option<Link> {
    let link = value.get("link")?;
    let target = text(link, "target")?;
    match link.get("type")?.as_str()? {
        "file" => Some(Link::File(target)),
        "section" => Some(Link::Section(target)),
        "issue" => Some(Link::Issue(target)),
        _ => None,
    }
}

/// Reads a diagram from the JSON an AI wrote. Every problem found is listed, so the reply can be fixed in one go.
pub fn parse_graph(json: &Value) -> Result<Graph, Vec<String>> {
    let mut problems = Vec::new();
    let kind = match json
        .get("type")
        .and_then(Value::as_str)
        .and_then(GraphKind::from_stored)
    {
        Some(kind) => kind,
        None => {
            problems.push("\"type\" must be component, flow, entity, or deployment.".to_string());
            GraphKind::Component
        }
    };
    let title = text(json, "title")
        .map(|title| short(&title))
        .unwrap_or_default();

    let mut groups = Vec::new();
    let mut group_ids = HashSet::new();
    for item in json
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match (text(item, "id"), text(item, "label")) {
            (Some(id), Some(label)) => {
                if group_ids.insert(id.clone()) {
                    groups.push(Group {
                        id,
                        label: short(&label),
                    });
                } else {
                    problems.push(format!("Group \"{id}\" appears twice."));
                }
            }
            _ => problems.push("Every group needs an \"id\" and a \"label\".".to_string()),
        }
    }

    let mut nodes = Vec::new();
    let mut node_ids = HashSet::new();
    for item in json
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(label)) = (text(item, "id"), text(item, "label")) else {
            problems.push("Every node needs an \"id\" and a \"label\".".to_string());
            continue;
        };
        if !node_ids.insert(id.clone()) {
            problems.push(format!("Node \"{id}\" appears twice."));
            continue;
        }
        let group = text(item, "group");
        if let Some(group) = &group
            && !group_ids.contains(group)
        {
            problems.push(format!(
                "Node \"{id}\" is in group \"{group}\", which is not listed."
            ));
        }
        nodes.push(Node {
            id,
            label: short(&label),
            kind: text(item, "kind").map(|kind| short(&kind)),
            group,
            link: link_of(item),
        });
    }
    if nodes.is_empty() {
        problems.push("A diagram needs at least one node.".to_string());
    }
    if nodes.len() > MAX_NODES {
        problems.push(format!(
            "A diagram may have at most {MAX_NODES} nodes. Split it into several."
        ));
    }

    let mut edges = Vec::new();
    for item in json
        .get("edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(from), Some(to)) = (text(item, "from"), text(item, "to")) else {
            problems.push("Every edge needs a \"from\" and a \"to\".".to_string());
            continue;
        };
        for end in [&from, &to] {
            if !node_ids.contains(end) {
                problems.push(format!("An edge refers to \"{end}\", which is not a node."));
            }
        }
        let style = match item.get("style").and_then(Value::as_str) {
            Some("dashed") => EdgeStyle::Dashed,
            _ => EdgeStyle::Solid,
        };
        edges.push(Edge {
            from,
            to,
            label: text(item, "label").map(|label| short(&label)),
            style,
        });
    }

    if problems.is_empty() {
        Ok(Graph {
            kind,
            title,
            nodes,
            edges,
            groups,
        })
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn valid() -> Value {
        json!({
            "type": "component",
            "title": "Overview",
            "groups": [{"id": "backend", "label": "Backend"}],
            "nodes": [
                {"id": "web", "label": "Web app", "kind": "client"},
                {"id": "api", "label": "API", "group": "backend", "link": {"type": "file", "target": "src/api.rs"}},
                {"id": "db", "label": "Database", "group": "backend"}
            ],
            "edges": [
                {"from": "web", "to": "api", "label": "HTTPS"},
                {"from": "api", "to": "db", "style": "dashed"}
            ]
        })
    }

    #[test]
    fn a_valid_graph_is_read_with_its_links_groups_and_styles() {
        let graph = parse_graph(&valid()).unwrap();
        assert_eq!(graph.kind, GraphKind::Component);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.nodes[1].link, Some(Link::File("src/api.rs".into())));
        assert_eq!(graph.nodes[1].group.as_deref(), Some("backend"));
        assert_eq!(graph.edges[1].style, EdgeStyle::Dashed);
        assert_eq!(graph.edges[0].label.as_deref(), Some("HTTPS"));
    }

    #[test]
    fn every_problem_is_listed_not_just_the_first() {
        let mut bad = valid();
        bad["type"] = json!("pie");
        bad["nodes"][1]["group"] = json!("missing");
        bad["edges"][0]["to"] = json!("ghost");
        bad["nodes"][2]["id"] = json!("api");
        let problems = parse_graph(&bad).unwrap_err();
        assert!(
            problems.iter().any(|p| p.contains("\"type\"")),
            "{problems:?}"
        );
        assert!(problems.iter().any(|p| p.contains("missing")));
        assert!(problems.iter().any(|p| p.contains("ghost")));
        assert!(problems.iter().any(|p| p.contains("appears twice")));
    }

    #[test]
    fn an_empty_or_oversized_diagram_is_refused() {
        assert!(parse_graph(&json!({"type": "flow", "nodes": []})).is_err());
        let nodes: Vec<Value> = (0..MAX_NODES + 1)
            .map(|n| json!({"id": format!("n{n}"), "label": "x"}))
            .collect();
        let problems = parse_graph(&json!({"type": "flow", "nodes": nodes})).unwrap_err();
        assert!(problems.iter().any(|p| p.contains("at most")));
    }

    #[test]
    fn long_labels_are_shortened_and_blank_ones_refused() {
        let long = "x".repeat(200);
        let graph =
            parse_graph(&json!({"type": "flow", "nodes": [{"id": "a", "label": long}]})).unwrap();
        assert_eq!(graph.nodes[0].label.chars().count(), MAX_LABEL);
        assert!(graph.nodes[0].label.ends_with('\u{2026}'));
        assert!(
            parse_graph(&json!({"type": "flow", "nodes": [{"id": "a", "label": "  "}]})).is_err()
        );
    }
}
