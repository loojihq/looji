//! Lays a graph out in layers, left to right. Each node goes in the layer after everything that points to it, so the
//! arrows mostly run one way. Nodes within a layer are ordered to keep edges short and members of a group together.
//! Cycles are allowed: an edge that closes a loop is drawn looping underneath. The result is plain numbers, so the
//! same layout draws in the app and in an SVG.

use std::collections::HashSet;

use crate::graph::Graph;

const MARGIN: f32 = 24.0;
const LAYER_GAP: f32 = 80.0;
const NODE_GAP: f32 = 28.0;
const GROUP_PAD: f32 = 16.0;
const GROUP_LABEL: f32 = 22.0;
const MIN_WIDTH: f32 = 96.0;
const CHAR_WIDTH: f32 = 7.4;
const HEIGHT: f32 = 44.0;
const HEIGHT_WITH_KIND: f32 = 58.0;

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    /// The node's place in `Graph::nodes`.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedGroup {
    /// The group's place in `Graph::groups`.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The path of one edge, as points to join in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// The edge's place in `Graph::edges`.
    pub edge: usize,
    pub points: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub nodes: Vec<PlacedNode>,
    pub routes: Vec<Route>,
    pub groups: Vec<PlacedGroup>,
    pub width: f32,
    pub height: f32,
}

fn node_size(graph: &Graph, index: usize) -> (f32, f32) {
    let node = &graph.nodes[index];
    let widest = node
        .label
        .chars()
        .count()
        .max(node.kind.as_ref().map_or(0, |kind| kind.chars().count()));
    (
        (widest as f32 * CHAR_WIDTH + 28.0).max(MIN_WIDTH),
        if node.kind.is_some() {
            HEIGHT_WITH_KIND
        } else {
            HEIGHT
        },
    )
}

/// The edges as index pairs that point forward, with any that close a cycle reversed.
fn forward_edges(graph: &Graph) -> Vec<(usize, usize)> {
    let count = graph.nodes.len();
    let position = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let pairs: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .filter_map(|edge| Some((position(&edge.from)?, position(&edge.to)?)))
        .filter(|(from, to)| from != to)
        .collect();
    let mut outgoing = vec![Vec::new(); count];
    for &(from, to) in &pairs {
        outgoing[from].push(to);
    }
    // A depth-first walk finds the edges that lead back to a node still being visited.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut marks = vec![Mark::New; count];
    let mut back: HashSet<(usize, usize)> = HashSet::new();
    for start in 0..count {
        if marks[start] != Mark::New {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        marks[start] = Mark::Open;
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if let Some(&target) = outgoing[node].get(*next) {
                *next += 1;
                match marks[target] {
                    Mark::New => {
                        marks[target] = Mark::Open;
                        stack.push((target, 0));
                    }
                    Mark::Open => {
                        back.insert((node, target));
                    }
                    Mark::Done => {}
                }
            } else {
                marks[node] = Mark::Done;
                stack.pop();
            }
        }
    }
    pairs
        .into_iter()
        .map(|(from, to)| {
            if back.contains(&(from, to)) {
                (to, from)
            } else {
                (from, to)
            }
        })
        .collect()
}

/// Places every node, route and group.
pub fn layout(graph: &Graph) -> Layout {
    let count = graph.nodes.len();
    if count == 0 {
        return Layout {
            nodes: Vec::new(),
            routes: Vec::new(),
            groups: Vec::new(),
            width: MARGIN * 2.0,
            height: MARGIN * 2.0,
        };
    }
    let forward = forward_edges(graph);

    // Each node sits one layer after the latest node that points to it.
    let mut rank = vec![0usize; count];
    for _ in 0..count {
        let mut changed = false;
        for &(from, to) in &forward {
            if rank[to] < rank[from] + 1 {
                rank[to] = rank[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let layer_count = rank.iter().max().copied().unwrap_or(0) + 1;
    let group_of = |index: usize| {
        graph.nodes[index]
            .group
            .as_ref()
            .and_then(|id| graph.groups.iter().position(|group| &group.id == id))
            .unwrap_or(usize::MAX)
    };
    // Each group, and the nodes outside any group, gets a horizontal lane of its own, so one group's box can never
    // run through another's. Lanes are ordered by how early their nodes sit.
    let mut lane_keys: Vec<usize> = (0..count).map(group_of).collect();
    lane_keys.sort_unstable();
    lane_keys.dedup();
    let mean_rank = |key: usize| {
        let members: Vec<usize> = (0..count)
            .filter(|&index| group_of(index) == key)
            .map(|index| rank[index])
            .collect();
        members.iter().sum::<usize>() as f32 / members.len() as f32
    };
    lane_keys.sort_by(|a, b| mean_rank(*a).total_cmp(&mean_rank(*b)).then(a.cmp(b)));
    let lane_of = |index: usize| {
        let key = group_of(index);
        lane_keys.iter().position(|&lane| lane == key).unwrap_or(0)
    };
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); layer_count];
    for index in 0..count {
        layers[rank[index]].push(index);
    }
    for layer in &mut layers {
        layer.sort_by_key(|&index| (lane_of(index), index));
    }

    // Order each layer by the average place of its neighbours, back and forth, so edges cross less.
    let mut place = vec![0.0f32; count];
    let refresh = |layers: &Vec<Vec<usize>>, place: &mut Vec<f32>| {
        for layer in layers {
            for (slot, &index) in layer.iter().enumerate() {
                place[index] = slot as f32;
            }
        }
    };
    refresh(&layers, &mut place);
    for sweep in 0..6 {
        let downward = sweep % 2 == 0;
        let order: Vec<usize> = if downward {
            (1..layer_count).collect()
        } else {
            (0..layer_count.saturating_sub(1)).rev().collect()
        };
        for layer_index in order {
            let mut keyed: Vec<(f32, usize, usize)> = layers[layer_index]
                .iter()
                .map(|&index| {
                    let neighbours: Vec<f32> = forward
                        .iter()
                        .filter_map(|&(from, to)| match (downward, to == index, from == index) {
                            (true, true, _) => Some(place[from]),
                            (false, _, true) => Some(place[to]),
                            _ => None,
                        })
                        .collect();
                    let centre = if neighbours.is_empty() {
                        place[index]
                    } else {
                        neighbours.iter().sum::<f32>() / neighbours.len() as f32
                    };
                    (centre, lane_of(index), index)
                })
                .collect();
            keyed.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.total_cmp(&b.0)).then(a.2.cmp(&b.2)));
            layers[layer_index] = keyed.into_iter().map(|(_, _, index)| index).collect();
            refresh(&layers, &mut place);
        }
    }

    // Sizes, then positions. Layers run left to right. Each lane is a band of the height its tallest layer needs, and
    // a group's box wraps its lane, so boxes of different groups are always apart.
    let sizes: Vec<(f32, f32)> = (0..count).map(|index| node_size(graph, index)).collect();
    let lane_count = lane_keys.len();
    let in_lane = |layer: &[usize], lane: usize| -> Vec<usize> {
        layer
            .iter()
            .copied()
            .filter(|&index| lane_of(index) == lane)
            .collect()
    };
    let stack_height = |members: &[usize]| -> f32 {
        members.iter().map(|&index| sizes[index].1).sum::<f32>()
            + NODE_GAP * members.len().saturating_sub(1) as f32
    };
    let mut lane_height = vec![0.0f32; lane_count];
    for layer in &layers {
        for (lane, height) in lane_height.iter_mut().enumerate() {
            *height = height.max(stack_height(&in_lane(layer, lane)));
        }
    }
    let is_group = |lane: usize| lane_keys[lane] != usize::MAX;
    let mut lane_top = vec![0.0f32; lane_count];
    let mut y = MARGIN;
    for lane in 0..lane_count {
        let (head, foot) = if is_group(lane) {
            (GROUP_PAD + GROUP_LABEL, GROUP_PAD)
        } else {
            (0.0, 0.0)
        };
        lane_top[lane] = y + head;
        y = lane_top[lane] + lane_height[lane] + foot + NODE_GAP;
    }
    let mut height = y - NODE_GAP + MARGIN;

    let mut nodes = vec![
        PlacedNode {
            index: 0,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        };
        count
    ];
    let mut x = MARGIN + GROUP_PAD;
    for layer in &layers {
        let widest = layer
            .iter()
            .map(|&index| sizes[index].0)
            .fold(0.0, f32::max);
        for lane in 0..lane_count {
            let members = in_lane(layer, lane);
            let mut top = lane_top[lane] + (lane_height[lane] - stack_height(&members)) / 2.0;
            for index in members {
                let (w, h) = sizes[index];
                nodes[index] = PlacedNode {
                    index,
                    x: x + (widest - w) / 2.0,
                    y: top,
                    w,
                    h,
                };
                top += h + NODE_GAP;
            }
        }
        x += widest + LAYER_GAP;
    }
    let mut width = x - LAYER_GAP + MARGIN + GROUP_PAD;

    // Groups wrap their lane, across the width their members span.
    let mut groups = Vec::new();
    for lane in 0..lane_count {
        if !is_group(lane) {
            continue;
        }
        let members: Vec<&PlacedNode> = nodes
            .iter()
            .filter(|placed| lane_of(placed.index) == lane)
            .collect();
        let left = members.iter().map(|m| m.x).fold(f32::MAX, f32::min) - GROUP_PAD;
        let right = members.iter().map(|m| m.x + m.w).fold(0.0, f32::max) + GROUP_PAD;
        let top = lane_top[lane] - GROUP_PAD - GROUP_LABEL;
        width = width.max(right + MARGIN);
        groups.push(PlacedGroup {
            index: lane_keys[lane],
            x: left,
            y: top,
            w: right - left,
            h: lane_height[lane] + GROUP_PAD * 2.0 + GROUP_LABEL,
        });
    }

    // Edges: straight to the next layer, and underneath for ones that go back or stay level.
    let position = |id: &str| graph.nodes.iter().position(|node| node.id == id);
    let loop_y = nodes.iter().map(|n| n.y + n.h).fold(0.0, f32::max) + 24.0;
    let mut routes = Vec::new();
    let mut below = false;
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let (Some(from), Some(to)) = (position(&edge.from), position(&edge.to)) else {
            continue;
        };
        let (a, b) = (&nodes[from], &nodes[to]);
        let points = if rank[to] > rank[from] && from != to {
            vec![(a.x + a.w, a.y + a.h / 2.0), (b.x, b.y + b.h / 2.0)]
        } else {
            below = true;
            vec![
                (a.x + a.w / 2.0, a.y + a.h),
                (a.x + a.w / 2.0, loop_y),
                (b.x + b.w / 2.0, loop_y),
                (b.x + b.w / 2.0, b.y + b.h),
            ]
        };
        routes.push(Route {
            edge: edge_index,
            points,
        });
    }
    if below {
        height = height.max(loop_y + MARGIN);
    }
    Layout {
        nodes,
        routes,
        groups,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::graph::parse_graph;

    fn graph(value: serde_json::Value) -> Graph {
        parse_graph(&value).unwrap()
    }

    fn chain() -> Graph {
        graph(json!({
            "type": "flow",
            "nodes": [{"id": "a", "label": "Start"}, {"id": "b", "label": "Middle"}, {"id": "c", "label": "End"}],
            "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}]
        }))
    }

    fn overlaps(a: &PlacedNode, b: &PlacedNode) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn a_chain_runs_left_to_right_with_straight_routes() {
        let placed = layout(&chain());
        assert!(placed.nodes[0].x < placed.nodes[1].x && placed.nodes[1].x < placed.nodes[2].x);
        assert_eq!(placed.routes.len(), 2);
        assert_eq!(placed.routes[0].points.len(), 2);
        assert!(placed.routes[0].points[0].0 < placed.routes[0].points[1].0);
    }

    #[test]
    fn no_two_nodes_overlap_and_everything_fits_the_canvas() {
        let g = graph(json!({
            "type": "component",
            "groups": [{"id": "g", "label": "Group"}],
            "nodes": [
                {"id": "a", "label": "A"}, {"id": "b", "label": "B", "group": "g"}, {"id": "c", "label": "C", "group": "g"},
                {"id": "d", "label": "D", "kind": "database"}, {"id": "e", "label": "A much longer label here"}
            ],
            "edges": [{"from": "a", "to": "b"}, {"from": "a", "to": "c"}, {"from": "b", "to": "d"}, {"from": "c", "to": "d"}, {"from": "e", "to": "d"}]
        }));
        let placed = layout(&g);
        for (i, first) in placed.nodes.iter().enumerate() {
            assert!(
                first.x >= 0.0
                    && first.y >= 0.0
                    && first.x + first.w <= placed.width
                    && first.y + first.h <= placed.height
            );
            for second in &placed.nodes[i + 1..] {
                assert!(!overlaps(first, second), "{first:?} overlaps {second:?}");
            }
        }
    }

    #[test]
    fn a_cycle_is_laid_out_and_its_closing_edge_loops_underneath() {
        let g = graph(json!({
            "type": "flow",
            "nodes": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}],
            "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "a"}]
        }));
        let placed = layout(&g);
        assert!(placed.nodes[0].x < placed.nodes[1].x);
        let looped = placed
            .routes
            .iter()
            .find(|route| route.points.len() == 4)
            .expect("one edge loops");
        assert!(looped.points[1].1 > placed.nodes[0].y + placed.nodes[0].h);
        assert!(placed.height > looped.points[1].1);
    }

    #[test]
    fn a_group_wraps_its_members() {
        let g = graph(json!({
            "type": "component",
            "groups": [{"id": "g", "label": "Backend"}],
            "nodes": [{"id": "a", "label": "A"}, {"id": "b", "label": "B", "group": "g"}, {"id": "c", "label": "C", "group": "g"}],
            "edges": [{"from": "a", "to": "b"}, {"from": "a", "to": "c"}]
        }));
        let placed = layout(&g);
        let group = &placed.groups[0];
        for node in placed
            .nodes
            .iter()
            .filter(|n| g.nodes[n.index].group.is_some())
        {
            assert!(node.x >= group.x && node.x + node.w <= group.x + group.w);
            assert!(node.y >= group.y && node.y + node.h <= group.y + group.h);
        }
    }

    #[test]
    fn groups_that_interleave_in_the_flow_still_never_overlap() {
        let g = graph(json!({
            "type": "component",
            "groups": [{"id": "x", "label": "X"}, {"id": "y", "label": "Y"}],
            "nodes": [
                {"id": "a", "label": "A", "group": "x"}, {"id": "b", "label": "B", "group": "y"},
                {"id": "c", "label": "C", "group": "x"}, {"id": "d", "label": "D", "group": "y"},
                {"id": "e", "label": "E"}
            ],
            "edges": [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}, {"from": "c", "to": "d"}, {"from": "e", "to": "c"}]
        }));
        let placed = layout(&g);
        assert_eq!(placed.groups.len(), 2);
        let (first, second) = (&placed.groups[0], &placed.groups[1]);
        let apart = first.y + first.h <= second.y || second.y + second.h <= first.y;
        assert!(apart, "{first:?} runs into {second:?}");
        for node in &placed.nodes {
            let source = &g.nodes[node.index];
            for group in &placed.groups {
                let inside = node.x >= group.x
                    && node.x + node.w <= group.x + group.w
                    && node.y >= group.y
                    && node.y + node.h <= group.y + group.h;
                let belongs = source.group.as_deref() == Some(g.groups[group.index].id.as_str());
                assert_eq!(
                    inside, belongs,
                    "{} and group {}",
                    source.id, g.groups[group.index].id
                );
            }
        }
    }

    #[test]
    fn layout_is_the_same_every_time() {
        assert_eq!(layout(&chain()), layout(&chain()));
    }

    #[test]
    fn an_edge_to_itself_does_not_hang_or_panic() {
        let g = graph(
            json!({"type": "flow", "nodes": [{"id": "a", "label": "A"}], "edges": [{"from": "a", "to": "a"}]}),
        );
        assert_eq!(layout(&g).nodes.len(), 1);
    }
}
