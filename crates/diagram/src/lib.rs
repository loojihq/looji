//! Diagrams as data. The AI never draws pixels: it writes a small graph (nodes, edges, groups), this crate checks it,
//! lays it out in layers, and exports it as Mermaid or SVG. The app draws the same layout natively, so a diagram
//! follows the theme and every node can link to a section, a file, or an issue.

mod export;
mod graph;
mod layout;

pub use export::{to_mermaid, to_png, to_svg};
pub use graph::{Edge, EdgeStyle, Graph, GraphKind, Group, Link, Node, parse_graph};
pub use layout::{Layout, PlacedGroup, PlacedNode, Route, layout};
