//! Export: Mermaid text, for READMEs and GitHub, and SVG, for anything that shows images. The SVG uses the same layout
//! the app draws, in neutral colours that read on a light or dark page.

use crate::{
    graph::{EdgeStyle, Graph, GraphKind},
    layout::layout,
};

fn mermaid_id(index: usize) -> String {
    format!("n{index}")
}

fn mermaid_text(text: &str) -> String {
    text.replace('"', "'").replace(['\n', '\r'], " ")
}

/// The diagram as Mermaid. Components, flows and deployments are flowcharts, with groups as subgraphs. Entities are an
/// entity-relationship diagram.
pub fn to_mermaid(graph: &Graph) -> String {
    let index_of = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let mut out = String::new();
    if graph.kind == GraphKind::Entity {
        out.push_str("erDiagram\n");
        for node in &graph.nodes {
            let name: String = node
                .label
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            out.push_str(&format!("    {name}\n"));
        }
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (index_of(&edge.from), index_of(&edge.to)) else {
                continue;
            };
            let name = |index: usize| -> String {
                graph.nodes[index]
                    .label
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect()
            };
            out.push_str(&format!(
                "    {} ||--o{{ {} : \"{}\"\n",
                name(from),
                name(to),
                mermaid_text(edge.label.as_deref().unwrap_or("relates to"))
            ));
        }
        return out;
    }
    out.push_str(if graph.kind == GraphKind::Deployment {
        "flowchart TB\n"
    } else {
        "flowchart LR\n"
    });
    let draw_node = |index: usize, indent: &str| {
        let node = &graph.nodes[index];
        let label = match &node.kind {
            Some(kind) => format!("{}<br/>({})", mermaid_text(&node.label), mermaid_text(kind)),
            None => mermaid_text(&node.label),
        };
        format!("{indent}{}[\"{label}\"]\n", mermaid_id(index))
    };
    for (group_index, group) in graph.groups.iter().enumerate() {
        out.push_str(&format!(
            "    subgraph g{group_index}[\"{}\"]\n",
            mermaid_text(&group.label)
        ));
        for (index, node) in graph.nodes.iter().enumerate() {
            if node.group.as_deref() == Some(group.id.as_str()) {
                out.push_str(&draw_node(index, "        "));
            }
        }
        out.push_str("    end\n");
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        let grouped = node
            .group
            .as_ref()
            .is_some_and(|id| graph.groups.iter().any(|group| &group.id == id));
        if !grouped {
            out.push_str(&draw_node(index, "    "));
        }
    }
    for edge in &graph.edges {
        let (Some(from), Some(to)) = (index_of(&edge.from), index_of(&edge.to)) else {
            continue;
        };
        let arrow = match (edge.style, edge.label.as_deref()) {
            (EdgeStyle::Solid, None) => "-->".to_string(),
            (EdgeStyle::Solid, Some(label)) => format!("-->|\"{}\"|", mermaid_text(label)),
            (EdgeStyle::Dashed, None) => "-.->".to_string(),
            (EdgeStyle::Dashed, Some(label)) => format!("-.->|\"{}\"|", mermaid_text(label)),
        };
        out.push_str(&format!(
            "    {} {arrow} {}\n",
            mermaid_id(from),
            mermaid_id(to)
        ));
    }
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The diagram as an SVG image.
pub fn to_svg(graph: &Graph) -> String {
    let placed = layout(graph);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.0} {h:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" font-family=\"sans-serif\" font-size=\"13\">\n\
         <defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#8a8f98\"/></marker></defs>\n",
        w = placed.width,
        h = placed.height
    );
    for group in &placed.groups {
        svg.push_str(&format!(
            "<rect x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\" rx=\"10\" fill=\"#8a8f98\" fill-opacity=\"0.08\" stroke=\"#8a8f98\" stroke-opacity=\"0.5\"/>\n<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#8a8f98\" font-size=\"12\">{}</text>\n",
            group.x,
            group.y,
            group.w,
            group.h,
            group.x + 10.0,
            group.y + 16.0,
            escape(&graph.groups[group.index].label)
        ));
    }
    for route in &placed.routes {
        let edge = &graph.edges[route.edge];
        let points: Vec<String> = route
            .points
            .iter()
            .map(|(x, y)| format!("{x:.0},{y:.0}"))
            .collect();
        let dash = if edge.style == EdgeStyle::Dashed {
            " stroke-dasharray=\"5 4\""
        } else {
            ""
        };
        svg.push_str(&format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"#8a8f98\" stroke-width=\"1.5\"{dash} marker-end=\"url(#arrow)\"/>\n",
            points.join(" ")
        ));
        if let Some(label) = &edge.label {
            let middle = route.points[route.points.len() / 2];
            let before = route.points[(route.points.len() / 2).saturating_sub(1)];
            svg.push_str(&format!(
                "<text x=\"{:.0}\" y=\"{:.0}\" fill=\"#8a8f98\" font-size=\"11\" text-anchor=\"middle\">{}</text>\n",
                (middle.0 + before.0) / 2.0,
                (middle.1 + before.1) / 2.0 - 4.0,
                escape(label)
            ));
        }
    }
    for node in &placed.nodes {
        let source = &graph.nodes[node.index];
        svg.push_str(&format!(
            "<rect x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\" rx=\"8\" fill=\"#f4f5f7\" stroke=\"#8a8f98\"/>\n",
            node.x, node.y, node.w, node.h
        ));
        let centre = node.x + node.w / 2.0;
        match &source.kind {
            Some(kind) => {
                svg.push_str(&format!(
                    "<text x=\"{centre:.0}\" y=\"{:.0}\" fill=\"#1d2025\" text-anchor=\"middle\">{}</text>\n<text x=\"{centre:.0}\" y=\"{:.0}\" fill=\"#6b7280\" font-size=\"10\" text-anchor=\"middle\">{}</text>\n",
                    node.y + node.h / 2.0 - 2.0,
                    escape(&source.label),
                    node.y + node.h / 2.0 + 14.0,
                    escape(kind)
                ));
            }
            None => svg.push_str(&format!(
                "<text x=\"{centre:.0}\" y=\"{:.0}\" fill=\"#1d2025\" text-anchor=\"middle\">{}</text>\n",
                node.y + node.h / 2.0 + 4.0,
                escape(&source.label)
            )),
        }
    }
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::graph::parse_graph;

    fn sample(kind: &str) -> Graph {
        parse_graph(&json!({
            "type": kind,
            "groups": [{"id": "g", "label": "Backend"}],
            "nodes": [
                {"id": "web", "label": "Web \"app\""},
                {"id": "api", "label": "API", "kind": "service", "group": "g"},
                {"id": "db", "label": "Orders & items", "group": "g"}
            ],
            "edges": [
                {"from": "web", "to": "api", "label": "HTTPS"},
                {"from": "api", "to": "db", "style": "dashed"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn a_component_diagram_is_a_flowchart_with_subgraphs_labels_and_dashes() {
        let text = to_mermaid(&sample("component"));
        assert!(text.starts_with("flowchart LR\n"));
        assert!(text.contains("subgraph g0[\"Backend\"]"));
        assert!(text.contains("n1[\"API<br/>(service)\"]"));
        assert!(text.contains("n0 -->|\"HTTPS\"| n1"));
        assert!(text.contains("n1 -.-> n2"));
        assert!(
            text.contains("Web 'app'"),
            "quotes cannot break the syntax: {text}"
        );
    }

    #[test]
    fn an_entity_diagram_is_an_er_diagram_with_safe_names() {
        let text = to_mermaid(&sample("entity"));
        assert!(text.starts_with("erDiagram\n"));
        assert!(text.contains("Orders___items"));
        assert!(text.contains("||--o{"));
    }

    #[test]
    fn a_deployment_diagram_runs_top_to_bottom() {
        assert!(to_mermaid(&sample("deployment")).starts_with("flowchart TB\n"));
    }

    #[test]
    fn the_svg_holds_every_node_label_and_escapes_markup() {
        let svg = to_svg(&sample("component"));
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains(">API<") && svg.contains(">service<") && svg.contains("HTTPS"));
        assert!(svg.contains("Orders &amp; items"));
        assert!(svg.contains("stroke-dasharray"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }
}
