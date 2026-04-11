//! d2-dagre-layout: bridge between d2-graph and the dagre-rs layout engine.
//!
//! Ported from Go `d2layouts/d2dagrelayout/layout.go`.

use std::collections::HashMap;

use d2_geo::{Point, Segment};
use d2_graph::{Graph, ObjId};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum rank separation (used in post-processing adjustments).
#[allow(dead_code)]
const MIN_RANK_SEP: i32 = 60;
const EDGE_LABEL_GAP: i32 = 20;
/// Default padding around container contents.
#[allow(dead_code)]
const DEFAULT_PADDING: f64 = 30.0;
/// Minimum spacing between elements.
#[allow(dead_code)]
const MIN_SPACING: f64 = 10.0;

// ---------------------------------------------------------------------------
// ConfigurableOpts
// ---------------------------------------------------------------------------

/// User-configurable layout options.
#[derive(Debug, Clone)]
pub struct ConfigurableOpts {
    pub node_sep: i32,
    pub edge_sep: i32,
}

impl Default for ConfigurableOpts {
    fn default() -> Self {
        Self {
            node_sep: 60,
            edge_sep: 20,
        }
    }
}

// ---------------------------------------------------------------------------
// ObjectMapper
// ---------------------------------------------------------------------------

/// Maps between d2 object indices and dagre node string IDs.
struct ObjectMapper {
    obj_to_id: HashMap<ObjId, String>,
    id_to_obj: HashMap<String, ObjId>,
    counter: usize,
}

impl ObjectMapper {
    fn new() -> Self {
        Self {
            obj_to_id: HashMap::new(),
            id_to_obj: HashMap::new(),
            counter: 0,
        }
    }

    fn register(&mut self, obj_id: ObjId) {
        let dagre_id = self.counter.to_string();
        self.obj_to_id.insert(obj_id, dagre_id.clone());
        self.id_to_obj.insert(dagre_id, obj_id);
        self.counter += 1;
    }

    fn to_dagre_id(&self, obj_id: ObjId) -> &str {
        &self.obj_to_id[&obj_id]
    }

    #[allow(dead_code)]
    fn to_obj_id(&self, dagre_id: &str) -> ObjId {
        self.id_to_obj[dagre_id]
    }
}

// ---------------------------------------------------------------------------
// Edge endpoint resolution (container -> descendant)
// ---------------------------------------------------------------------------

/// Find the effective endpoints for an edge, routing through containers.
/// dagre cannot handle edges to containers, so we route to leaf descendants.
fn get_edge_endpoints(g: &Graph, edge_idx: usize) -> (ObjId, ObjId) {
    let edge = &g.edges[edge_idx];
    let mut src = edge.src;
    let mut dst = edge.dst;

    // Route container edges to their first/last child
    while g.objects[src].is_container()
        && g.objects[src].class.is_none()
        && g.objects[src].sql_table.is_none()
    {
        src = get_longest_edge_chain_tail(g, src);
    }
    while g.objects[dst].is_container()
        && g.objects[dst].class.is_none()
        && g.objects[dst].sql_table.is_none()
    {
        dst = get_longest_edge_chain_head(g, dst);
    }

    // For reverse arrows (b <- a), swap endpoints
    if edge.src_arrow && !edge.dst_arrow {
        std::mem::swap(&mut src, &mut dst);
    }
    (src, dst)
}

/// Check if `obj` is equal to or a descendant of `container`.
fn in_container(obj_id: ObjId, container_id: ObjId, g: &Graph) -> Option<ObjId> {
    in_container_depth(obj_id, container_id, g, 0)
}

fn in_container_depth(
    obj_id: ObjId,
    container_id: ObjId,
    g: &Graph,
    depth: usize,
) -> Option<ObjId> {
    if depth > 100 {
        return None; // prevent infinite recursion from cyclic parent pointers
    }
    if obj_id == container_id {
        return Some(obj_id);
    }
    if g.objects[obj_id].parent == Some(container_id) {
        return Some(obj_id);
    }
    if let Some(parent) = g.objects[obj_id].parent {
        return in_container_depth(parent, container_id, g, depth + 1);
    }
    None
}

/// Get the head of the longest edge chain in a container (first child in chain).
fn get_longest_edge_chain_head(g: &Graph, container: ObjId) -> ObjId {
    let children = &g.objects[container].children_array;
    if children.is_empty() {
        return container;
    }

    let mut rank: HashMap<ObjId, i32> = HashMap::new();
    let mut chain_length: HashMap<ObjId, i32> = HashMap::new();

    for &child in children {
        let mut is_head = true;
        for edge in &g.edges {
            if in_container(edge.src, container, g).is_some()
                && in_container(edge.dst, child, g).is_some()
            {
                is_head = false;
                break;
            }
        }
        if !is_head {
            continue;
        }
        rank.insert(child, 1);
        chain_length.insert(child, 1);

        // BFS to find chain length
        let mut queue = vec![child];
        let mut visited = std::collections::HashSet::new();
        while let Some(curr) = queue.first().copied() {
            queue.remove(0);
            if !visited.insert(curr) {
                continue;
            }
            for edge in &g.edges {
                let dst_child = in_container(edge.dst, container, g);
                if dst_child == Some(curr) {
                    continue;
                }
                if let Some(dc) = dst_child {
                    if in_container(edge.src, curr, g).is_some() {
                        let new_rank = rank.get(&curr).copied().unwrap_or(0) + 1;
                        if new_rank > rank.get(&dc).copied().unwrap_or(0) {
                            rank.insert(dc, new_rank);
                            let cl = chain_length.entry(child).or_insert(0);
                            *cl = (*cl).max(new_rank);
                        }
                        queue.push(dc);
                    }
                }
            }
        }
    }

    let max_chain = children
        .iter()
        .filter_map(|c| chain_length.get(c))
        .copied()
        .max()
        .unwrap_or(0);

    let heads: Vec<ObjId> = children
        .iter()
        .filter(|&&c| {
            rank.get(&c).copied().unwrap_or(0) == 1
                && chain_length.get(&c).copied().unwrap_or(0) == max_chain
        })
        .copied()
        .collect();

    if !heads.is_empty() {
        heads[heads.len() / 2]
    } else {
        children[0]
    }
}

/// Get the tail of the longest edge chain in a container.
fn get_longest_edge_chain_tail(g: &Graph, container: ObjId) -> ObjId {
    let children = &g.objects[container].children_array;
    if children.is_empty() {
        return container;
    }

    let mut rank: HashMap<ObjId, i32> = HashMap::new();

    for &child in children {
        let mut is_head = true;
        for edge in &g.edges {
            if in_container(edge.src, container, g).is_some()
                && in_container(edge.dst, child, g).is_some()
            {
                is_head = false;
                break;
            }
        }
        if !is_head {
            continue;
        }
        rank.insert(child, 1);

        // BFS
        let mut queue = vec![child];
        let mut visited = std::collections::HashSet::new();
        while let Some(curr) = queue.first().copied() {
            queue.remove(0);
            if !visited.insert(curr) {
                continue;
            }
            for edge in &g.edges {
                let dst_child = in_container(edge.dst, container, g);
                if dst_child == Some(curr) {
                    continue;
                }
                if let Some(dc) = dst_child {
                    if in_container(edge.src, curr, g).is_some() {
                        let new_rank = rank.get(&curr).copied().unwrap_or(0) + 1;
                        let old = rank.get(&dc).copied().unwrap_or(0);
                        rank.insert(dc, old.max(new_rank));
                        queue.push(dc);
                    }
                }
            }
        }
    }

    let max_rank = children
        .iter()
        .filter_map(|c| rank.get(c))
        .copied()
        .max()
        .unwrap_or(0);

    let tails: Vec<ObjId> = children
        .iter()
        .filter(|&&c| rank.get(&c).copied().unwrap_or(0) == max_rank)
        .copied()
        .collect();

    if !tails.is_empty() {
        tails[tails.len() / 2]
    } else {
        children[0]
    }
}

// ---------------------------------------------------------------------------
// Label/icon positioning
// ---------------------------------------------------------------------------

/// Set default label and icon positions for an object.
fn position_labels_icons(obj: &mut d2_graph::Object) {
    if obj.icon.is_some() && obj.icon_position.is_none() {
        if !obj.children_array.is_empty() {
            obj.icon_position = Some("OUTSIDE_TOP_LEFT".to_owned());
            if obj.label_position.is_none() {
                obj.label_position = Some("OUTSIDE_TOP_RIGHT".to_owned());
                return;
            }
        } else if obj.sql_table.is_some() || obj.class.is_some() || !obj.language.is_empty() {
            obj.icon_position = Some("OUTSIDE_TOP_LEFT".to_owned());
        } else {
            obj.icon_position = Some("INSIDE_MIDDLE_CENTER".to_owned());
        }
    }

    if obj.has_label() && obj.label_position.is_none() {
        if !obj.children_array.is_empty() {
            obj.label_position = Some("OUTSIDE_TOP_CENTER".to_owned());
        } else if obj.has_outside_bottom_label() {
            obj.label_position = Some("OUTSIDE_BOTTOM_CENTER".to_owned());
        } else if obj.icon.is_some() {
            obj.label_position = Some("INSIDE_TOP_CENTER".to_owned());
        } else {
            obj.label_position = Some("INSIDE_MIDDLE_CENTER".to_owned());
        }

        if (obj.label_dimensions.width as f64) > obj.width
            || (obj.label_dimensions.height as f64) > obj.height
        {
            if !obj.children_array.is_empty() {
                obj.label_position = Some("OUTSIDE_TOP_CENTER".to_owned());
            } else {
                obj.label_position = Some("OUTSIDE_BOTTOM_CENTER".to_owned());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Layout entry point
// ---------------------------------------------------------------------------

/// Run the dagre layout algorithm on a d2 graph.
///
/// This builds a dagre graph from d2 objects and edges, runs the layout,
/// then reads back positions and routes.
pub fn layout(g: &mut Graph, opts: Option<&ConfigurableOpts>) -> Result<(), String> {
    let default_opts = ConfigurableOpts::default();
    let opts = opts.unwrap_or(&default_opts);

    // Determine direction
    let root_direction = g.root_obj().direction.value.clone();
    let is_horizontal = matches!(root_direction.as_str(), "right" | "left");
    let rankdir = match root_direction.as_str() {
        "right" => dagre::RankDir::LR,
        "left" => dagre::RankDir::RL,
        "up" => dagre::RankDir::BT,
        _ => dagre::RankDir::TB,
    };

    // Position labels and icons
    for i in 0..g.objects.len() {
        if i == g.root {
            continue;
        }
        // Borrow-safe: copy needed data, modify object
        position_labels_icons(&mut g.objects[i]);
    }

    // Compute max label dimensions for rank separation
    let mut max_label_width = 0;
    let mut max_label_height = 0;
    for edge in &g.edges {
        max_label_width = max_label_width.max(edge.label_dimensions.width);
        max_label_height = max_label_height.max(edge.label_dimensions.height);
    }

    let ranksep = if !is_horizontal {
        100i32.max(max_label_height + 40)
    } else {
        100i32.max(max_label_width + 40)
    };

    // Build dagre graph
    let graph_opts = dagre::graph::GraphOptions {
        directed: true,
        multigraph: true,
        compound: true,
    };
    let mut dagre_g =
        dagre::graph::Graph::<dagre::NodeLabel, dagre::EdgeLabel>::with_options(graph_opts);

    // Set graph-level label
    dagre_g.set_graph_label(dagre::GraphLabel {
        compound: true,
        rankdir,
        nodesep: opts.node_sep as f64,
        edgesep: opts.edge_sep as f64,
        ranksep: ranksep as f64,
        ..Default::default()
    });

    // Register all objects with the mapper
    let mut mapper = ObjectMapper::new();
    let obj_ids: Vec<ObjId> = (0..g.objects.len()).filter(|&i| i != g.root).collect();
    for &obj_id in &obj_ids {
        mapper.register(obj_id);
    }

    // Add nodes
    for &obj_id in &obj_ids {
        let obj = &g.objects[obj_id];
        let dagre_id = mapper.to_dagre_id(obj_id).to_owned();
        dagre_g.set_node(
            dagre_id,
            Some(dagre::NodeLabel {
                width: obj.width,
                height: obj.height,
                ..Default::default()
            }),
        );
    }

    // Set parents for compound graph
    for &obj_id in &obj_ids {
        let parent = g.objects[obj_id].parent;
        if let Some(parent_id) = parent {
            if parent_id != g.root {
                let child_dagre = mapper.to_dagre_id(obj_id).to_owned();
                let parent_dagre = mapper.to_dagre_id(parent_id).to_owned();
                dagre_g.set_parent(&child_dagre, Some(&parent_dagre));
            }
        }
    }

    // Add edges
    // Collect edge endpoint data first (immutable borrow)
    let edge_data: Vec<(ObjId, ObjId, i32, i32, String)> = (0..g.edges.len())
        .map(|ei| {
            let (src, dst) = get_edge_endpoints(g, ei);
            let edge = &g.edges[ei];
            let mut width = edge.label_dimensions.width;
            let mut height = edge.label_dimensions.height;

            // Count parallel edges for gap spacing
            let num_parallel = g
                .edges
                .iter()
                .filter(|e2| {
                    let (s2, d2) = (e2.src, e2.dst);
                    // Simplified: check direct src/dst match
                    (s2 == edge.src && d2 == edge.dst) || (s2 == edge.dst && d2 == edge.src)
                })
                .count();

            if num_parallel > 1 {
                match root_direction.as_str() {
                    "left" | "right" => height += EDGE_LABEL_GAP,
                    _ => width += EDGE_LABEL_GAP,
                }
            }

            let abs_id = edge.abs_id.clone();
            (src, dst, width, height, abs_id)
        })
        .collect();

    for (src, dst, width, height, abs_id) in &edge_data {
        let src_dagre = mapper.to_dagre_id(*src).to_owned();
        let dst_dagre = mapper.to_dagre_id(*dst).to_owned();
        dagre_g.set_edge(
            src_dagre,
            dst_dagre,
            Some(dagre::EdgeLabel {
                width: *width as f64,
                height: *height as f64,
                labelpos: dagre::layout::types::LabelPos::Center,
                ..Default::default()
            }),
            Some(abs_id.as_str()),
        );
    }

    // Run layout
    let layout_opts = dagre::LayoutOptions {
        rankdir,
        nodesep: opts.node_sep as f64,
        edgesep: opts.edge_sep as f64,
        ranksep: ranksep as f64,
        ..Default::default()
    };
    dagre::layout(&mut dagre_g, Some(layout_opts));

    // Read back node positions
    for &obj_id in &obj_ids {
        let dagre_id = mapper.to_dagre_id(obj_id);
        if let Some(node_label) = dagre_g.node(dagre_id) {
            if let (Some(cx), Some(cy)) = (node_label.x, node_label.y) {
                let w = node_label.width;
                let h = node_label.height;
                let obj = &mut g.objects[obj_id];
                // dagre gives center coordinates; convert to top-left
                obj.top_left = Point::new((cx - w / 2.0).round(), (cy - h / 2.0).round());
                obj.width = w.ceil();
                obj.height = h.ceil();
                obj.update_box();
            }
        }
    }

    // Read back edge routes
    let dagre_edges = dagre_g.edges();
    for (ei, edge_desc) in dagre_edges.iter().enumerate() {
        if ei >= g.edges.len() {
            break;
        }

        if let Some(edge_label) = dagre_g.edge_by_obj(edge_desc) {
            let raw_points: Vec<Point> = edge_label
                .points
                .iter()
                .map(|p| Point::new(p.x, p.y))
                .collect();

            if raw_points.is_empty() {
                continue;
            }

            let edge = &g.edges[ei];

            // Reverse points for reverse arrows
            let mut points: Vec<Point> = if edge.src_arrow && !edge.dst_arrow {
                raw_points.into_iter().rev().collect()
            } else {
                raw_points
            };

            // Clip route at source/destination bounding boxes
            let src_obj = &g.objects[edge.src];
            let dst_obj = &g.objects[edge.dst];

            if edge.src != edge.dst && points.len() >= 2 {
                let mut start_index = 0;
                let mut end_index = points.len() - 1;
                let mut start = points[0];
                let mut end = points[end_index];

                for i in 1..points.len() {
                    let seg = Segment::new(points[i - 1], points[i]);
                    let ints = src_obj.box_.intersections(&seg);
                    if !ints.is_empty() {
                        start = ints[0];
                        start_index = i - 1;
                    }
                    let ints = dst_obj.box_.intersections(&seg);
                    if !ints.is_empty() {
                        end = ints[0];
                        end_index = i;
                        break;
                    }
                }

                points = points[start_index..=end_index].to_vec();
                points[0] = start;
                let last = points.len() - 1;
                points[last] = end;
            }

            // Build curved path from route points
            let mut path = Vec::new();
            if points.len() > 2 {
                // Build vectors between consecutive points
                let vectors: Vec<d2_geo::Vector> = (1..points.len())
                    .map(|i| points[i - 1].vector_to(&points[i]))
                    .collect();

                path.push(points[0]);
                if vectors.len() > 1 {
                    path.push(points[0].add_vector(&vectors[0].multiply(0.8)));
                    for i in 1..vectors.len().saturating_sub(1) {
                        let p = points[i];
                        let v = &vectors[i];
                        path.push(p.add_vector(&v.multiply(0.2)));
                        path.push(p.add_vector(&v.multiply(0.5)));
                        path.push(p.add_vector(&v.multiply(0.8)));
                    }
                    let last_pt_idx = points.len() - 2;
                    let last_vec_idx = vectors.len() - 1;
                    path.push(points[last_pt_idx].add_vector(&vectors[last_vec_idx].multiply(0.2)));
                    g.edges[ei].is_curve = true;
                }
                path.push(points[points.len() - 1]);
            } else {
                path = points;
            }

            g.edges[ei].route = path;

            // Set edge label position
            if !g.edges[ei].label.value.is_empty() {
                g.edges[ei].label_position = Some("INSIDE_MIDDLE_CENTER".to_owned());
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use d2_graph::{Edge, Object};

    /// Simple 2-node graph layout test.
    #[test]
    fn layout_two_nodes() {
        let mut g = Graph::new();
        let a = g.add_object(Object {
            id: "a".into(),
            abs_id: "a".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        let b = g.add_object(Object {
            id: "b".into(),
            abs_id: "b".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        g.add_edge(Edge {
            abs_id: "(a -> b)[0]".into(),
            src: a,
            dst: b,
            ..Default::default()
        });

        let result = layout(&mut g, None);
        assert!(result.is_ok(), "layout failed: {:?}", result.err());

        // After layout, objects should have positions
        let obj_a = &g.objects[a];
        let obj_b = &g.objects[b];

        // Both should have non-zero or at least assigned positions
        // In TB mode, b should be below a
        assert!(
            obj_b.top_left.y > obj_a.top_left.y,
            "b ({}) should be below a ({})",
            obj_b.top_left.y,
            obj_a.top_left.y
        );

        // Edge should have route points
        assert!(
            !g.edges[0].route.is_empty(),
            "edge should have route points"
        );
    }

    /// Test with configurable opts.
    #[test]
    fn layout_with_custom_opts() {
        let mut g = Graph::new();
        let a = g.add_object(Object {
            id: "a".into(),
            abs_id: "a".into(),
            width: 80.0,
            height: 40.0,
            ..Default::default()
        });
        let b = g.add_object(Object {
            id: "b".into(),
            abs_id: "b".into(),
            width: 80.0,
            height: 40.0,
            ..Default::default()
        });
        g.add_edge(Edge {
            abs_id: "(a -> b)[0]".into(),
            src: a,
            dst: b,
            ..Default::default()
        });

        let opts = ConfigurableOpts {
            node_sep: 100,
            edge_sep: 40,
        };
        let result = layout(&mut g, Some(&opts));
        assert!(result.is_ok());
    }

    /// Test with parent-child (container) relationships.
    #[test]
    fn layout_with_container() {
        let mut g = Graph::new();
        let parent = g.add_object(Object {
            id: "container".into(),
            abs_id: "container".into(),
            width: 200.0,
            height: 150.0,
            ..Default::default()
        });
        let child = g.add_object(Object {
            id: "child".into(),
            abs_id: "container.child".into(),
            parent: Some(parent),
            width: 80.0,
            height: 40.0,
            ..Default::default()
        });
        let other = g.add_object(Object {
            id: "other".into(),
            abs_id: "other".into(),
            width: 80.0,
            height: 40.0,
            ..Default::default()
        });
        g.add_edge(Edge {
            abs_id: "(child -> other)[0]".into(),
            src: child,
            dst: other,
            ..Default::default()
        });

        let result = layout(&mut g, None);
        assert!(result.is_ok(), "layout failed: {:?}", result.err());
    }

    /// Test horizontal layout direction.
    #[test]
    fn layout_horizontal() {
        let mut g = Graph::new();
        g.root_obj_mut().direction.value = "right".to_owned();
        let a = g.add_object(Object {
            id: "a".into(),
            abs_id: "a".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        let b = g.add_object(Object {
            id: "b".into(),
            abs_id: "b".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        g.add_edge(Edge {
            abs_id: "(a -> b)[0]".into(),
            src: a,
            dst: b,
            ..Default::default()
        });

        let result = layout(&mut g, None);
        assert!(result.is_ok());

        // In LR mode, b should be to the right of a
        let obj_a = &g.objects[a];
        let obj_b = &g.objects[b];
        assert!(
            obj_b.top_left.x > obj_a.top_left.x,
            "b ({}) should be right of a ({})",
            obj_b.top_left.x,
            obj_a.top_left.x
        );
    }

    /// Test layout with an edge that has labels.
    #[test]
    fn layout_edge_with_label() {
        let mut g = Graph::new();
        let a = g.add_object(Object {
            id: "a".into(),
            abs_id: "a".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        let b = g.add_object(Object {
            id: "b".into(),
            abs_id: "b".into(),
            width: 100.0,
            height: 50.0,
            ..Default::default()
        });
        g.add_edge(Edge {
            abs_id: "(a -> b)[0]".into(),
            src: a,
            dst: b,
            label: d2_graph::Label {
                value: "connects".into(),
                map_key: None,
            },
            label_dimensions: d2_graph::Dimensions {
                width: 60,
                height: 16,
            },
            ..Default::default()
        });

        let result = layout(&mut g, None);
        assert!(result.is_ok());

        // Label position should be set
        assert_eq!(
            g.edges[0].label_position.as_deref(),
            Some("INSIDE_MIDDLE_CENTER")
        );
    }
}
