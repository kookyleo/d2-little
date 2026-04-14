//! d2-lib: top-level entry point that ties the entire d2-little pipeline together.
//!
//! Pipeline: D2 source text -> AST -> IR -> Graph -> (theme + dimensions + layout) -> Diagram -> SVG.
//!
//! Ported from Go `d2lib/d2.go`.

use std::collections::{HashMap, HashSet};

use d2_fonts::{self, FONT_SIZE_M, FontFamily, FontStyle};
use d2_geo::Point;
use d2_graph::{self, Graph, ObjId};
use d2_svg_render::{self, RenderOpts};
use d2_target;
use d2_textmeasure;
use d2_themes;

// ---------------------------------------------------------------------------
// Constants (matching Go d2graph constants)
// ---------------------------------------------------------------------------

const DEFAULT_SHAPE_SIZE: f64 = 100.0;
const MIN_SHAPE_SIZE: f64 = 5.0;
/// Padding added around label text inside a shape (Go d2graph.INNER_LABEL_PADDING = 5).
const INNER_LABEL_PADDING: f64 = 5.0;
/// Default shape padding (matches Go lib/shape baseShape.defaultPadding = 40).
const DEFAULT_SHAPE_PADDING: f64 = 40.0;

fn has_none_text_transform(style: &d2_graph::Style) -> bool {
    style
        .text_transform
        .as_ref()
        .is_some_and(|v| v.value == "none")
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if at_word_start {
                out.extend(ch.to_uppercase());
                at_word_start = false;
            } else {
                out.extend(ch.to_lowercase());
            }
        } else {
            at_word_start = true;
            out.push(ch);
        }
    }
    out
}

fn apply_text_transform(
    label: &str,
    style: &d2_graph::Style,
    caps_lock: bool,
    skip_caps_lock: bool,
) -> String {
    let mut out = label.to_string();
    if caps_lock && !skip_caps_lock && !has_none_text_transform(style) {
        out = out.to_uppercase();
    }
    if let Some(transform) = style.text_transform.as_ref().map(|v| v.value.as_str()) {
        out = match transform {
            "uppercase" => out.to_uppercase(),
            "lowercase" => out.to_lowercase(),
            "capitalize" => title_case(&out),
            _ => out,
        };
    }
    out
}

// ---------------------------------------------------------------------------
// CompileOptions
// ---------------------------------------------------------------------------

/// Options controlling the compile phase.
pub struct CompileOptions {
    pub ruler: Option<d2_textmeasure::Ruler>,
    pub theme_id: Option<i64>,
    pub dark_theme_id: Option<i64>,
    pub pad: Option<i64>,
    pub sketch: bool,
    pub center: bool,
    pub layout_engine: String,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            ruler: None,
            theme_id: None,
            dark_theme_id: None,
            pad: None,
            sketch: false,
            center: false,
            layout_engine: "dagre".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse D2 source text into an AST.
pub fn parse(input: &str) -> Result<d2_ast::Map, String> {
    let (ast_map, parse_err) = d2_parser::parse("", input);
    if let Some(e) = parse_err {
        return Err(format!("{}", e));
    }
    Ok(ast_map)
}

/// Compile D2 source text into a diagram and SVG bytes.
///
/// Steps:
/// 1. Parse & compile source text into a Graph
/// 2. Apply theme
/// 3. Set dimensions (text measurement)
/// 4. Run dagre layout
/// 5. Export to Diagram
/// 6. Render to SVG
pub fn compile(
    input: &str,
    opts: &CompileOptions,
) -> Result<(d2_target::Diagram, Vec<u8>), String> {
    // Step 1: parse + IR + compile -> Graph
    let (mut g, config) =
        d2_compiler::compile_with_config("", input).map_err(|e| format!("{}", e))?;

    let mut theme_id = opts.theme_id;
    let mut dark_theme_id = opts.dark_theme_id;
    let mut pad = opts.pad;
    let mut center = opts.center;
    let mut sketch = opts.sketch;
    let mut theme_overrides = None;
    let mut dark_theme_overrides = None;
    let mut config_data = std::collections::HashMap::new();

    if let Some(config) = config.as_ref() {
        if theme_id.is_none() {
            theme_id = config.theme_id;
        }
        if dark_theme_id.is_none() {
            dark_theme_id = config.dark_theme_id;
        }
        if pad.is_none() {
            pad = config.pad;
        }
        if !center {
            center = config.center.unwrap_or(false);
        }
        if !sketch {
            sketch = config.sketch.unwrap_or(false);
        }
        theme_overrides = config.theme_overrides.clone();
        dark_theme_overrides = config.dark_theme_overrides.clone();
        config_data = config.data.clone();
    }

    let theme_id = theme_id.unwrap_or(0);

    // Step 2-5: recursively compile graph (theme, dimensions, layout, export)
    let mut ruler = d2_textmeasure::Ruler::new().map_err(|e| format!("ruler init: {}", e))?;
    let mut diagram = compile_graph(&mut g, theme_id, &mut ruler)?;

    // Match Go d2lib.Compile: copy selected render options back into
    // diagram.Config so the diagram hash (used for CSS scoping) accounts for
    // appearance-affecting fields like themeID and sketch.
    // Go d2lib.Compile feeds the original parsed config back into
    // diagram.Config after overwriting ThemeID/DarkThemeID/Sketch with
    // the resolved render options. The remaining fields (pad, center,
    // layoutEngine) keep their original parsed values.
    diagram.config = Some(d2_target::Config {
        sketch: Some(sketch),
        theme_id: Some(theme_id),
        dark_theme_id,
        pad: config.as_ref().and_then(|c| c.pad),
        center: config.as_ref().and_then(|c| c.center),
        layout_engine: config.as_ref().and_then(|c| c.layout_engine.clone()),
        theme_overrides,
        dark_theme_overrides,
        data: config_data,
    });

    // Step 6: render
    //
    // Mirrors the Go e2e pipeline (`d2/e2etests/e2e_test.go`):
    //   1. RenderMultiboard -> boards ([][]byte)
    //   2. If len(boards) == 1, return boards[0]
    //   3. Else call d2animate.Wrap(diagram, boards, opts, 1000)
    // When the diagram has nested boards, set MasterID on opts so inner SVGs
    // use <g> form rather than standalone <svg>.
    let mut render_opts = RenderOpts {
        theme_id: Some(theme_id),
        dark_theme_id,
        pad,
        sketch: if sketch { Some(true) } else { None },
        center: if center { Some(true) } else { None },
        theme_overrides: diagram
            .config
            .as_ref()
            .and_then(|c| c.theme_overrides.clone()),
        dark_theme_overrides: diagram
            .config
            .as_ref()
            .and_then(|c| c.dark_theme_overrides.clone()),
        ..Default::default()
    };

    if !diagram.layers.is_empty() || !diagram.scenarios.is_empty() || !diagram.steps.is_empty() {
        // Multi-board: use the root hash for CSS targeting across all boards.
        render_opts.master_id = diagram.hash_id(None);
    }

    let boards = d2_svg_render::render_multiboard(&diagram, &render_opts)?;

    let svg = if boards.len() == 1 {
        boards.into_iter().next().unwrap()
    } else {
        d2_svg_render::wrap(&diagram, &boards, &render_opts, 1000)?
    };

    Ok((diagram, svg))
}

/// Recursively compile a graph into a diagram: apply theme, set dimensions,
/// run layout, export, then recurse into layers/scenarios/steps.
/// Mirrors Go d2lib.compile.
fn compile_graph(
    g: &mut Graph,
    theme_id: i64,
    ruler: &mut d2_textmeasure::Ruler,
) -> Result<d2_target::Diagram, String> {
    // Apply theme
    if let Some(theme) = d2_themes::catalog::find(theme_id) {
        g.theme = Some(theme.clone());
    }

    if g.objects.len() > 1 || !g.edges.is_empty() {
        // Set dimensions
        set_dimensions(g, ruler)?;

        // Layout with nested diagram support
        layout_nested(g)?;
    }

    // Export
    let mut diagram = d2_exporter::export(g, None, None)?;

    // Recursively compile nested boards
    for layer in &mut g.layers {
        let ld = compile_graph(layer, theme_id, ruler)?;
        diagram.layers.push(ld);
    }
    for scenario in &mut g.scenarios {
        let sd = compile_graph(scenario, theme_id, ruler)?;
        diagram.scenarios.push(sd);
    }
    for step in &mut g.steps {
        let sd = compile_graph(step, theme_id, ruler)?;
        diagram.steps.push(sd);
    }

    Ok(diagram)
}

// ---------------------------------------------------------------------------
// layout_nested: handle nested sequence/grid diagrams before main layout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct SubObjResult {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    label_position: Option<String>,
    label: d2_graph::Label,
    shape: d2_graph::ScalarValue,
    z_index: i32,
    is_sequence_diagram_note: bool,
    is_sequence_diagram_group: bool,
}

#[derive(Debug, Clone)]
struct NestedResult {
    container_id: ObjId,
    obj_results: HashMap<ObjId, SubObjResult>,
    edge_routes: HashMap<usize, (Vec<Point>, Option<String>, i32)>, // (route, label_position, z_index)
    new_edges: Vec<d2_graph::Edge>,
    container_width: f64,
    container_height: f64,
    container_label_position: Option<String>,
    container_icon_position: Option<String>,
}

fn layout_container_as_subgraph(g: &Graph, container_id: ObjId) -> Result<NestedResult, String> {
    let mut sub_g = Graph::new();
    sub_g.root_level = g.objects[container_id].level(g);

    let mut root_copy = g.objects[container_id].clone();
    root_copy.parent = None;
    root_copy.children.clear();
    root_copy.children_array.clear();
    sub_g.objects[sub_g.root] = root_copy;

    let mut id_map: HashMap<ObjId, ObjId> = HashMap::new();
    id_map.insert(container_id, sub_g.root);

    let children: Vec<ObjId> = g.objects[container_id].children_array.clone();
    let mut queue: std::collections::VecDeque<ObjId> = children.iter().copied().collect();
    while let Some(obj_id) = queue.pop_front() {
        let obj = &g.objects[obj_id];
        let mut new_obj = obj.clone();
        let new_id = sub_g.objects.len();

        new_obj.children.clear();
        new_obj.children_array.clear();

        let parent_main_id = obj.parent.unwrap_or(container_id);
        new_obj.parent = Some(*id_map.get(&parent_main_id).unwrap_or(&sub_g.root));
        id_map.insert(obj_id, new_id);
        sub_g.objects.push(new_obj);

        let parent_sub_id = *id_map.get(&parent_main_id).unwrap_or(&sub_g.root);
        sub_g.objects[parent_sub_id].children.push(new_id);
        sub_g.objects[parent_sub_id].children_array.push(new_id);

        for &child_id in &g.objects[obj_id].children_array {
            queue.push_back(child_id);
        }
    }

    for i in 0..sub_g.objects.len() {
        for r in &mut sub_g.objects[i].references {
            if let Some(scope) = r.scope_obj {
                r.scope_obj = id_map.get(&scope).copied();
            }
        }
    }

    let mut edge_map: HashMap<usize, usize> = HashMap::new();
    for (ei, edge) in g.edges.iter().enumerate() {
        if let (Some(&sub_src), Some(&sub_dst)) = (id_map.get(&edge.src), id_map.get(&edge.dst)) {
            let mut new_edge = edge.clone();
            new_edge.src = sub_src;
            new_edge.dst = sub_dst;
            if let Some(scope) = new_edge.scope_obj {
                new_edge.scope_obj = id_map.get(&scope).copied();
            }
            let new_ei = sub_g.edges.len();
            edge_map.insert(ei, new_ei);
            sub_g.edges.push(new_edge);
        }
    }

    layout_nested(&mut sub_g)?;

    let mut obj_results = HashMap::new();
    for (&main_id, &sub_id) in &id_map {
        if main_id == container_id {
            continue;
        }
        let obj = &sub_g.objects[sub_id];
        obj_results.insert(
            main_id,
            SubObjResult {
                x: obj.top_left.x,
                y: obj.top_left.y,
                w: obj.width,
                h: obj.height,
                label_position: obj.label_position.clone(),
                label: obj.label.clone(),
                shape: obj.shape.clone(),
                z_index: obj.z_index,
                is_sequence_diagram_note: obj.is_sequence_diagram_note,
                is_sequence_diagram_group: obj.is_sequence_diagram_group,
            },
        );
    }

    let mut edge_routes: HashMap<usize, (Vec<Point>, Option<String>, i32)> = HashMap::new();
    let mapped_sub_indices: HashSet<usize> = edge_map.values().copied().collect();
    for (&main_ei, &sub_ei) in &edge_map {
        let sub_edge = &sub_g.edges[sub_ei];
        edge_routes.insert(
            main_ei,
            (
                sub_edge.route.clone(),
                sub_edge.label_position.clone(),
                sub_edge.z_index,
            ),
        );
    }

    let reverse_id_map: HashMap<ObjId, ObjId> = id_map.iter().map(|(&m, &s)| (s, m)).collect();
    let mut new_edges: Vec<d2_graph::Edge> = Vec::new();
    for (sub_ei, sub_edge) in sub_g.edges.iter().enumerate() {
        if !mapped_sub_indices.contains(&sub_ei) {
            let mut edge = sub_edge.clone();
            if let Some(&main_src) = reverse_id_map.get(&edge.src) {
                edge.src = main_src;
            }
            if let Some(&main_dst) = reverse_id_map.get(&edge.dst) {
                edge.dst = main_dst;
            }
            if let Some(scope) = edge.scope_obj {
                edge.scope_obj = reverse_id_map.get(&scope).copied();
            }
            new_edges.push(edge);
        }
    }

    let root = &sub_g.objects[sub_g.root];
    Ok(NestedResult {
        container_id,
        obj_results,
        edge_routes,
        new_edges,
        container_width: root.width,
        container_height: root.height,
        container_label_position: root.label_position.clone(),
        container_icon_position: root.icon_position.clone(),
    })
}

fn apply_nested_object_results(g: &mut Graph, result: &NestedResult, dx: f64, dy: f64) {
    for (&obj_id, res) in &result.obj_results {
        let obj = &mut g.objects[obj_id];
        obj.top_left = Point::new(res.x + dx, res.y + dy);
        obj.width = res.w;
        obj.height = res.h;
        obj.label_position = res.label_position.clone();
        obj.label = res.label.clone();
        obj.shape = res.shape.clone();
        obj.z_index = res.z_index;
        obj.is_sequence_diagram_note = res.is_sequence_diagram_note;
        obj.is_sequence_diagram_group = res.is_sequence_diagram_group;
        obj.update_box();
    }
}

fn apply_nested_edge_results(g: &mut Graph, result: &NestedResult, dx: f64, dy: f64) {
    for (&ei, (route, label_pos, z_index)) in &result.edge_routes {
        let edge = &mut g.edges[ei];
        edge.route = route
            .iter()
            .map(|p| Point::new(p.x + dx, p.y + dy))
            .collect();
        if let Some(pos) = label_pos {
            edge.label_position = Some(pos.clone());
        }
        edge.z_index = *z_index;
    }

    for mut edge in result.new_edges.clone() {
        for p in &mut edge.route {
            p.x += dx;
            p.y += dy;
        }
        g.edges.push(edge);
    }
}

fn remove_edges_touching_descendants(
    g: &mut Graph,
    excluded_descendants: &HashSet<ObjId>,
) -> Vec<(usize, d2_graph::Edge)> {
    let mut saved_edges: Vec<(usize, d2_graph::Edge)> = Vec::new();
    let mut removed_indices: Vec<usize> = g
        .edges
        .iter()
        .enumerate()
        .filter_map(|(ei, edge)| {
            (excluded_descendants.contains(&edge.src) || excluded_descendants.contains(&edge.dst))
                .then_some(ei)
        })
        .collect();
    removed_indices.sort_unstable_by(|a, b| b.cmp(a));
    for &ei in &removed_indices {
        saved_edges.push((ei, g.edges.remove(ei)));
    }
    saved_edges
}

fn restore_removed_edges(g: &mut Graph, mut saved_edges: Vec<(usize, d2_graph::Edge)>) {
    saved_edges.reverse();
    for (ei, edge) in saved_edges {
        g.edges.insert(ei, edge);
    }
}

fn move_obj_with_descendants_and_boxes(g: &mut Graph, obj_id: ObjId, dx: f64, dy: f64) {
    if obj_id >= g.objects.len() {
        return;
    }
    g.objects[obj_id].top_left.x += dx;
    g.objects[obj_id].top_left.y += dy;
    g.objects[obj_id].update_box();
    let children: Vec<ObjId> = g.objects[obj_id].children_array.clone();
    for child_id in children {
        move_obj_with_descendants_and_boxes(g, child_id, dx, dy);
    }
}

/// Mirrors Go d2layouts.LayoutNested. Before running the main layout engine,
/// detect children that are sequence diagrams, extract them, run sequence layout,
/// fit them to their containers, then run the main dagre layout, and finally
/// offset nested contents to their container positions.
fn layout_nested(g: &mut Graph) -> Result<(), String> {
    if g.root_obj().is_sequence_diagram() {
        let root = g.root;
        let nested_children: Vec<ObjId> = g.objects[root]
            .children_array
            .iter()
            .copied()
            .filter(|&child_id| {
                !g.objects[child_id].children_array.is_empty()
                    && (g.objects[child_id].is_grid_diagram()
                        || g.objects[child_id].is_sequence_diagram())
            })
            .collect();

        if nested_children.is_empty() {
            return d2_sequence::layout(g);
        }

        let mut nested_results = Vec::new();
        let mut excluded_descendants: HashSet<ObjId> = HashSet::new();
        let saved_children: Vec<(ObjId, Vec<ObjId>, Vec<ObjId>)> = nested_children
            .iter()
            .map(|&child_id| {
                let result = layout_container_as_subgraph(g, child_id)?;
                nested_results.push(result);
                collect_descendants(g, child_id, &mut excluded_descendants);
                Ok((
                    child_id,
                    g.objects[child_id].children.clone(),
                    g.objects[child_id].children_array.clone(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;

        for result in &nested_results {
            g.objects[result.container_id].width = result.container_width;
            g.objects[result.container_id].height = result.container_height;
            if let Some(ref pos) = result.container_label_position {
                g.objects[result.container_id].label_position = Some(pos.clone());
            }
            if let Some(ref pos) = result.container_icon_position {
                g.objects[result.container_id].icon_position = Some(pos.clone());
            }
        }

        for (child_id, _, _) in &saved_children {
            g.objects[*child_id].children.clear();
            g.objects[*child_id].children_array.clear();
        }

        let saved_edges = remove_edges_touching_descendants(g, &excluded_descendants);
        d2_sequence::layout(g)?;

        for (child_id, children, children_array) in saved_children {
            g.objects[child_id].children = children;
            g.objects[child_id].children_array = children_array;
        }
        restore_removed_edges(g, saved_edges);

        for result in &nested_results {
            let dx = g.objects[result.container_id].top_left.x;
            let dy = g.objects[result.container_id].top_left.y;
            apply_nested_object_results(g, result, dx, dy);
            apply_nested_edge_results(g, result, dx, dy);
        }
        route_direct_edges_for_excluded_descendants(g, &excluded_descendants);
        return Ok(());
    }

    if g.root_obj().is_grid_diagram() {
        let root = g.root;
        let nested_children: Vec<ObjId> = g.objects[root]
            .children_array
            .iter()
            .copied()
            .filter(|&child_id| !g.objects[child_id].children_array.is_empty())
            .collect();

        if nested_children.is_empty() {
            return d2_grid::layout(g);
        }

        let mut nested_results = Vec::new();
        let mut excluded_descendants: HashSet<ObjId> = HashSet::new();
        let saved_children: Vec<(ObjId, Vec<ObjId>, Vec<ObjId>)> = nested_children
            .iter()
            .map(|&child_id| {
                let result = layout_container_as_subgraph(g, child_id)?;
                nested_results.push(result);
                collect_descendants(g, child_id, &mut excluded_descendants);
                Ok((
                    child_id,
                    g.objects[child_id].children.clone(),
                    g.objects[child_id].children_array.clone(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;

        for result in &nested_results {
            g.objects[result.container_id].width = result.container_width;
            g.objects[result.container_id].height = result.container_height;
            if let Some(ref pos) = result.container_label_position {
                g.objects[result.container_id].label_position = Some(pos.clone());
            }
            if let Some(ref pos) = result.container_icon_position {
                g.objects[result.container_id].icon_position = Some(pos.clone());
            }
        }

        for (child_id, _, _) in &saved_children {
            g.objects[*child_id].children.clear();
            g.objects[*child_id].children_array.clear();
        }

        let saved_edges = remove_edges_touching_descendants(g, &excluded_descendants);
        d2_grid::layout(g)?;

        for (child_id, children, children_array) in saved_children {
            g.objects[child_id].children = children;
            g.objects[child_id].children_array = children_array;
        }
        restore_removed_edges(g, saved_edges);

        for result in &nested_results {
            let dx = g.objects[result.container_id].top_left.x;
            let dy = g.objects[result.container_id].top_left.y;
            apply_nested_object_results(g, result, dx, dy);
            apply_nested_edge_results(g, result, dx, dy);
        }
        route_direct_edges_for_excluded_descendants(g, &excluded_descendants);
        return Ok(());
    }

    // Find all non-root objects that are sequence or grid diagrams.
    let seq_containers: Vec<ObjId> = (0..g.objects.len())
        .filter(|&i| {
            i != g.root
                && g.objects[i].is_sequence_diagram()
                // Match Go LayoutNested: empty nested sequence diagrams stay in
                // the main graph and are laid out as normal shapes.
                && !g.objects[i].children_array.is_empty()
        })
        .collect();

    // Find grid containers that need pre-layout
    let grid_containers: Vec<ObjId> = (0..g.objects.len())
        .filter(|&i| {
            i != g.root
                && g.objects[i].is_grid_diagram()
                && !g.objects[i].children_array.is_empty()
        })
        .collect();

    // Pre-layout grid containers: for each grid container, build a temporary
    // sub-graph and run grid layout on it, then set the container's dimensions.
    for &container_id in &grid_containers {
        let children: Vec<ObjId> = g.objects[container_id].children_array.clone();
        if children.is_empty() {
            continue;
        }

        for &child_id in &children {
            if g.objects[child_id].children_array.is_empty() {
                continue;
            }
            let result = layout_container_as_subgraph(g, child_id)?;
            g.objects[result.container_id].width = result.container_width;
            g.objects[result.container_id].height = result.container_height;
            if let Some(ref pos) = result.container_label_position {
                g.objects[result.container_id].label_position = Some(pos.clone());
            }
            if let Some(ref pos) = result.container_icon_position {
                g.objects[result.container_id].icon_position = Some(pos.clone());
            }
            apply_nested_object_results(g, &result, 0.0, 0.0);
        }

        // Build a temporary sub-graph for this grid container.
        let mut sub = Graph::new();
        sub.root_level = g.objects[container_id].level(g);

        // Map original ObjIds to sub-graph ObjIds.
        let mut id_map: HashMap<ObjId, ObjId> = HashMap::new();
        id_map.insert(container_id, sub.root);

        // Copy root properties.
        sub.objects[sub.root].grid_rows = g.objects[container_id].grid_rows.clone();
        sub.objects[sub.root].grid_columns = g.objects[container_id].grid_columns.clone();
        sub.objects[sub.root].grid_gap = g.objects[container_id].grid_gap.clone();
        sub.objects[sub.root].vertical_gap = g.objects[container_id].vertical_gap.clone();
        sub.objects[sub.root].horizontal_gap = g.objects[container_id].horizontal_gap.clone();
        sub.objects[sub.root].label = g.objects[container_id].label.clone();
        sub.objects[sub.root].label_dimensions = g.objects[container_id].label_dimensions;
        sub.objects[sub.root].label_position = g.objects[container_id].label_position.clone();
        sub.objects[sub.root].icon = g.objects[container_id].icon.clone();
        sub.objects[sub.root].icon_position = g.objects[container_id].icon_position.clone();
        sub.objects[sub.root].shape = g.objects[container_id].shape.clone();
        sub.objects[sub.root].width = g.objects[container_id].width;
        sub.objects[sub.root].height = g.objects[container_id].height;
        sub.objects[sub.root].width_attr = g.objects[container_id].width_attr.clone();
        sub.objects[sub.root].height_attr = g.objects[container_id].height_attr.clone();
        // Leave sub.root.top_left at (0,0) so the nested grid layout positions its
        // contents relative to (0,0). The container's actual top_left is re-applied
        // later (see grid_children_map post-processing).
        sub.objects[sub.root].top_left = Point::new(0.0, 0.0);
        sub.objects[sub.root].style = g.objects[container_id].style.clone();

        // Add children to sub-graph.
        // Clear children_array because ObjIds point into main graph, not sub-graph.
        // Grid layout only needs each cell's outer dimensions.
        for &child_id in &children {
            let new_id = sub.objects.len();
            let mut child_copy = g.objects[child_id].clone();
            child_copy.parent = Some(sub.root);
            let was_container = !child_copy.children_array.is_empty();
            child_copy.children_array.clear();
            // Preserve container knowledge for label positioning. Only set
            // this for containers with a non-empty label; Go's grid layout
            // guards the same defaulting with `o.HasLabel()`, so containers
            // like `TALA: ""` must stay without a label position.
            if was_container && child_copy.label_position.is_none() && child_copy.has_label() {
                child_copy.label_position = Some("OUTSIDE_TOP_CENTER".to_owned());
            }
            sub.objects.push(child_copy);
            sub.objects[sub.root].children_array.push(new_id);
            id_map.insert(child_id, new_id);
        }

        // Run grid layout on the sub-graph.
        d2_grid::layout(&mut sub)?;

        // Copy results back to the main graph.
        g.objects[container_id].width = sub.objects[sub.root].width;
        g.objects[container_id].height = sub.objects[sub.root].height;
        g.objects[container_id].label_position = sub.objects[sub.root].label_position.clone();
        g.objects[container_id].icon_position = sub.objects[sub.root].icon_position.clone();

        // Copy child positions and sizes back (positions relative to container origin).
        for &child_id in &children {
            if let Some(&sub_id) = id_map.get(&child_id) {
                g.objects[child_id].top_left = sub.objects[sub_id].top_left;
                g.objects[child_id].width = sub.objects[sub_id].width;
                g.objects[child_id].height = sub.objects[sub_id].height;
                g.objects[child_id].label_position =
                    sub.objects[sub_id].label_position.clone();
                g.objects[child_id].icon_position =
                    sub.objects[sub_id].icon_position.clone();
            }
        }

        // Mark grid children as removed so dagre skips them.
        // After dagre, we restore them and offset to container position.
        g.objects[container_id].children_array.clear();
    }

    // Collect all grid descendants so dagre can skip them.
    let mut grid_all_descendants: HashSet<ObjId> = HashSet::new();
    let mut grid_children_map: HashMap<ObjId, Vec<ObjId>> = HashMap::new();
    for &container_id in &grid_containers {
        let direct_children: Vec<ObjId> = (0..g.objects.len())
            .filter(|&i| g.objects[i].parent == Some(container_id))
            .collect();
        for &child_id in &direct_children {
            grid_all_descendants.insert(child_id);
            collect_descendants(g, child_id, &mut grid_all_descendants);
        }
        // Temporarily clear children_array so dagre sees container as leaf
        grid_children_map.insert(container_id, direct_children);
        g.objects[container_id].children_array.clear();
    }

    if seq_containers.is_empty() && grid_containers.is_empty() {
        return d2_dagre_layout::layout(g, None);
    }

    if seq_containers.is_empty() {
        // Run dagre with grid descendants excluded.
        d2_dagre_layout::layout_with_exclude(g, None, &grid_all_descendants)?;

        // Restore children and offset grid cells to container positions.
        for (&container_id, children) in &grid_children_map {
            g.objects[container_id].children_array = children.clone();
            let dx = g.objects[container_id].top_left.x;
            let dy = g.objects[container_id].top_left.y;
            if dx != 0.0 || dy != 0.0 {
                for &child_id in children {
                    move_obj_with_descendants_and_boxes(g, child_id, dx, dy);
                }
            }
        }
        route_direct_edges_for_excluded_descendants(g, &grid_all_descendants);
        return Ok(());
    }

    // Collect all descendants of sequence diagram containers.
    let mut seq_descendants: HashSet<ObjId> = HashSet::new();
    for &container_id in &seq_containers {
        collect_descendants(g, container_id, &mut seq_descendants);
    }

    let mut nested_results: Vec<NestedResult> = Vec::new();

    for &container_id in &seq_containers {
        nested_results.push(layout_container_as_subgraph(g, container_id)?);
    }

    // Apply nested layout results: set container sizes, label positions, and child positions.
    for result in &nested_results {
        g.objects[result.container_id].width = result.container_width;
        g.objects[result.container_id].height = result.container_height;
        if let Some(ref pos) = result.container_label_position {
            g.objects[result.container_id].label_position = Some(pos.clone());
        }
        if let Some(ref pos) = result.container_icon_position {
            g.objects[result.container_id].icon_position = Some(pos.clone());
        }
    }

    // Run dagre layout on the main graph, excluding sequence diagram internals.
    // Mark descendants with sentinel shape so dagre skips them, and clear
    // container children so dagre treats containers as leaf nodes.
    let sentinel = "__d2_seq_nested_removed__";

    // Save and modify: container children + descendant shapes.
    let saved_children: Vec<(ObjId, Vec<ObjId>, Vec<ObjId>)> = seq_containers
        .iter()
        .map(|&c| {
            let children = g.objects[c].children.clone();
            let children_array = g.objects[c].children_array.clone();
            g.objects[c].children.clear();
            g.objects[c].children_array.clear();
            (c, children, children_array)
        })
        .collect();

    let saved_shapes: Vec<(ObjId, String)> = seq_descendants
        .iter()
        .map(|&d| {
            let old = g.objects[d].shape.value.clone();
            g.objects[d].shape.value = sentinel.to_string();
            (d, old)
        })
        .collect();

    // Save and remove internal/external edges touching sequence descendants.
    let saved_edges = remove_edges_touching_descendants(g, &seq_descendants);

    d2_dagre_layout::layout_with_exclude(g, None, &grid_all_descendants)?;

    // Restore container children.
    for (c, children, children_array) in saved_children {
        g.objects[c].children = children;
        g.objects[c].children_array = children_array;
    }

    // Restore descendant shapes.
    for (d, shape) in saved_shapes {
        g.objects[d].shape.value = shape;
    }

    // Restore edges.
    restore_removed_edges(g, saved_edges);

    // Now offset nested sequence diagram contents by their container's position,
    // and add newly created edges (e.g. lifelines) to the main graph.
    for result in nested_results {
        let container = &g.objects[result.container_id];
        let dx = container.top_left.x;
        let dy = container.top_left.y;

        apply_nested_object_results(g, &result, dx, dy);
        apply_nested_edge_results(g, &result, dx, dy);
    }

    // Restore grid children and offset grid cells to their container positions.
    for (&container_id, children) in &grid_children_map {
        g.objects[container_id].children_array = children.clone();
        let dx = g.objects[container_id].top_left.x;
        let dy = g.objects[container_id].top_left.y;
        if dx != 0.0 || dy != 0.0 {
            for &child_id in children {
                move_obj_with_descendants_and_boxes(g, child_id, dx, dy);
            }
        }
    }

    let mut excluded_special_descendants = grid_all_descendants.clone();
    excluded_special_descendants.extend(seq_descendants.iter().copied());
    route_direct_edges_for_excluded_descendants(g, &excluded_special_descendants);

    Ok(())
}

/// Collect all descendants of an object (not including the object itself).
fn collect_descendants(g: &Graph, obj_id: ObjId, out: &mut HashSet<ObjId>) {
    for &child_id in &g.objects[obj_id].children_array {
        out.insert(child_id);
        collect_descendants(g, child_id, out);
    }
}

fn route_direct_edges_for_excluded_descendants(g: &mut Graph, excluded_descendants: &HashSet<ObjId>) {
    for ei in 0..g.edges.len() {
        let edge = &g.edges[ei];
        if !edge.route.is_empty() {
            continue;
        }
        if !excluded_descendants.contains(&edge.src) && !excluded_descendants.contains(&edge.dst) {
            continue;
        }

        let src = g.objects[edge.src].center();
        let dst = g.objects[edge.dst].center();
        let mut points = vec![src, dst];
        let (new_start, new_end) = edge.trace_to_shape(&points, 0, 1, g);
        points = points[new_start..=new_end].to_vec();

        if points.len() >= 2 {
            let src_box = g.objects[edge.src].box_;
            let dst_box = g.objects[edge.dst].box_;
            let last = points.len() - 1;
            let starting_segment = d2_geo::Segment::new(points[1], points[0]);
            let ending_segment = d2_geo::Segment::new(points[last - 1], points[last]);

            if let Some(p) = src_box.intersections(&starting_segment).first().copied() {
                points[0] = p;
            }
            if let Some(p) = dst_box.intersections(&ending_segment).first().copied() {
                points[last] = p;
            }
        }

        let edge = &mut g.edges[ei];
        edge.route = points;
        if !edge.label.value.is_empty() {
            edge.label_position = Some("INSIDE_MIDDLE_CENTER".to_owned());
        }
    }
}

/// Convenience function: D2 source text -> SVG bytes with default options.
///
/// Uses pad=0 and the multi-board + animate wrapper pipeline to match the
/// Go e2e test output byte-for-byte (see `d2/e2etests/e2e_test.go` which
/// calls `d2animate.Wrap` when `len(boards) != 1`).
pub fn d2_to_svg(input: &str) -> Result<Vec<u8>, String> {
    let opts = CompileOptions {
        pad: Some(0),
        ..CompileOptions::default()
    };
    let (_, svg) = compile(input, &opts)?;
    Ok(svg)
}

// ---------------------------------------------------------------------------
// set_dimensions: measure text and assign object/edge dimensions
// ---------------------------------------------------------------------------

/// Measure label text for each object and edge, then set their width/height.
///
/// This is a simplified port of Go's `Graph.SetDimensions`.
pub fn set_dimensions(g: &mut Graph, ruler: &mut d2_textmeasure::Ruler) -> Result<(), String> {
    // Default font family for the diagram. Themes with the `mono` special
    // rule (e.g. the terminal theme) force everything to mono; otherwise
    // start from SourceSansPro and let per-object `style.font: mono` opt
    // individual labels into mono. Mirrors Go d2graph.GetLabelSize.
    let caps_lock = g.theme.as_ref().is_some_and(|t| t.special_rules.caps_lock);
    let default_family = if g.theme.as_ref().is_some_and(|t| t.special_rules.mono) {
        FontFamily::SourceCodePro
    } else {
        FontFamily::SourceSansPro
    };

    let measure_label = |ruler: &mut d2_textmeasure::Ruler,
                         shape: &str,
                         language: &str,
                         font_family: FontFamily,
                         font: d2_fonts::Font,
                         font_size: i32,
                         label: &str|
     -> Result<(i32, i32), String> {
        // Code shapes with an explicit language go through the mono
        // ruler path in Go `GetTextDimensionsWithMono`. The label is
        // measured in SourceCodePro at CODE_LINE_HEIGHT, then Go adds
        // a vertical fudge for leading/trailing blank lines that the
        // ruler cannot account for on its own.
        if !language.is_empty() && shape == d2_target::SHAPE_CODE {
            let original_lh = ruler.line_height_factor;
            ruler.line_height_factor = d2_textmeasure::CODE_LINE_HEIGHT;
            let mono_font = d2_fonts::Font::new(
                FontFamily::SourceCodePro,
                d2_fonts::FontStyle::Regular,
                font_size,
            );
            let (w, mut h) = ruler.measure_mono(mono_font, label);
            ruler.line_height_factor = original_lh;

            // Leading / trailing empty lines: Go counts them separately
            // because `MeasureMono` strips them from the bounds. A leading
            // blank line adds one font-size tall row, and each trailing
            // blank line adds `CODE_LINE_HEIGHT * font_size` rounded up.
            let lines: Vec<&str> = label.split('\n').collect();
            let has_leading =
                !lines.is_empty() && lines.first().map(|l| l.trim().is_empty()).unwrap_or(false);
            let mut num_trailing = 0usize;
            for l in lines.iter().rev() {
                if l.trim().is_empty() {
                    num_trailing += 1;
                } else {
                    break;
                }
            }
            if has_leading && num_trailing < lines.len() {
                h += font_size;
            }
            h += (d2_textmeasure::CODE_LINE_HEIGHT * f64::from(font_size * num_trailing as i32))
                .ceil() as i32;
            return Ok((w, h));
        }
        if language == "latex" {
            d2_latex::measure(label).map_err(|e| format!("latex measure: {}", e))
        } else if language == "markdown" {
            d2_textmeasure::measure_markdown(
                label,
                ruler,
                Some(font_family),
                Some(FontFamily::SourceCodePro),
                font_size,
            )
        } else if !language.is_empty() {
            // Non-code shapes with a non-markdown language are still
            // treated as markdown by Go (see GetLabelSize).
            d2_textmeasure::measure_markdown(
                label,
                ruler,
                Some(font_family),
                Some(FontFamily::SourceCodePro),
                font_size,
            )
        } else {
            Ok(ruler.measure(font, label))
        }
    };

    // Process objects (skip root at index 0)
    let count = g.objects.len();
    for i in 1..count {
        g.objects[i].label.value = apply_text_transform(
            &g.objects[i].label.value,
            &g.objects[i].style,
            caps_lock,
            g.objects[i]
                .shape
                .value
                .eq_ignore_ascii_case(d2_target::SHAPE_CODE)
                || g.objects[i].language == "latex",
        );
        let label = g.objects[i].label.value.clone();
        let shape = g.objects[i].shape.value.clone();
        // Match Go d2graph.GetLabelSize: if the object has `style.font`,
        // resolve it through the d2fonts.D2_FONT_TO_FAMILY map (only "mono"
        // is currently meaningful — anything else stays on the default
        // family).
        let font_family = match g.objects[i].style.font.as_ref().map(|v| v.value.as_str()) {
            Some("mono") => FontFamily::SourceCodePro,
            _ => default_family,
        };

        // Parse desired dimensions from user attributes
        let desired_width: i32 = g.objects[i]
            .width_attr
            .as_ref()
            .and_then(|v| v.value.parse().ok())
            .unwrap_or(0);
        let desired_height: i32 = g.objects[i]
            .height_attr
            .as_ref()
            .and_then(|v| v.value.parse().ok())
            .unwrap_or(0);

        // Determine font style.
        // Match Go d2graph.Object.Text(): leaf shapes (not container, not "text"
        // shape) default to bold; explicit style.bold can override.
        // Inside sequence diagrams all objects get isBold=false (Go:
        // `if obj.OuterSequenceDiagram() != nil { isBold = false }`).
        let is_container = !g.objects[i].children_array.is_empty();
        let is_grid = g.objects[i].is_grid_diagram();
        let in_seq = g.objects[i].is_inside_sequence_diagram(g);
        let mut is_bold = !is_container && shape != "text";
        if let Some(v) = g.objects[i].style.bold.as_ref() {
            is_bold = v.value == "true";
        }
        if in_seq {
            is_bold = false;
        }
        let is_italic = g.objects[i]
            .style
            .italic
            .as_ref()
            .is_some_and(|v| v.value == "true");
        // Default font size is FONT_SIZE_M (16). Containers and grid
        // diagrams get a level-based size that scales with depth:
        // level 1 → XXL, 2 → XL, 3 → L, else M. An explicit
        // `style.font-size` always wins. Mirrors Go
        // d2graph.Object.Text() + ContainerLevel.LabelSize().
        let font_size: i32 = if let Some(v) = g.objects[i].style.font_size.as_ref() {
            v.value.parse().unwrap_or(FONT_SIZE_M)
        } else if !in_seq && (is_container || is_grid) && shape != "text" {
            let level = g.objects[i].level(g);
            match level {
                1 => d2_fonts::FONT_SIZE_XXL,
                2 => d2_fonts::FONT_SIZE_XL,
                3 => d2_fonts::FONT_SIZE_L,
                _ => FONT_SIZE_M,
            }
        } else {
            FONT_SIZE_M
        };

        let font_style = if is_bold {
            FontStyle::Bold
        } else if is_italic {
            FontStyle::Italic
        } else {
            FontStyle::Regular
        };

        let font = d2_fonts::Font::new(font_family, font_style, font_size);

        // Class shapes need per-row sizing so the header + fields +
        // methods all fit. Mirrors Go `d2graph.GetDefaultSize` class
        // branch.
        if shape == "class" {
            // Go uses FONT_SIZE_L (20) by default for class measurements,
            // not the general FONT_SIZE_M (16).
            let class_font_size = if let Some(v) = g.objects[i].style.font_size.as_ref() {
                v.value.parse().unwrap_or(d2_fonts::FONT_SIZE_L)
            } else {
                d2_fonts::FONT_SIZE_L
            };
            let header_font_size = class_font_size + d2_target::HEADER_FONT_ADD;
            // Go `GetLabelSize` uses `GetTextDimensionsWithMono` with the
            // mono font for class shapes — the label is measured in mono
            // even though Text() reports `isBold=false` / fontFamily=default.
            let header_font = d2_fonts::Font::new(
                d2_fonts::FontFamily::SourceCodePro,
                FontStyle::Regular,
                header_font_size,
            );
            let (header_w, header_h) = if !label.is_empty() {
                ruler.measure(header_font, &label)
            } else {
                (0, 0)
            };
            g.objects[i].label_dimensions = d2_graph::Dimensions {
                width: header_w,
                height: header_h,
            };

            // Go's GetDefaultSize adds INNER_LABEL_PADDING to labelDims
            // when withLabelPadding is true (no explicit dims and non-empty
            // label). Apply the same adjustment to header_w/header_h.
            let with_label_padding = desired_width == 0 && desired_height == 0 && !label.is_empty();
            let label_pad = if with_label_padding {
                INNER_LABEL_PADDING as i32
            } else {
                0
            };
            let padded_header_w = header_w + label_pad;
            let padded_header_h = header_h + label_pad;

            // Row measurements use mono font at `class_font_size`, and Go
            // measures the full row text `Name + Type` concatenated (not
            // the pieces individually).
            let row_font = d2_fonts::Font::new(
                d2_fonts::FontFamily::SourceCodePro,
                FontStyle::Regular,
                class_font_size,
            );
            let mut max_width = 12i32.max(padded_header_w);
            let mut row_h = 0i32;

            let class_ref_opt = g.objects[i].class.clone();
            if let Some(ref cls) = class_ref_opt {
                for f in &cls.fields {
                    let combined = format!("{}{}", f.name, f.type_);
                    let (fw, fh) = ruler.measure(row_font, &combined);
                    max_width = max_width.max(fw);
                    row_h = row_h.max(fh);
                }
                for m in &cls.methods {
                    let combined = format!("{}{}", m.name, m.return_);
                    let (mw, mh) = ruler.measure(row_font, &combined);
                    max_width = max_width.max(mw);
                    row_h = row_h.max(mh);
                }
            }

            let w = d2_target::PREFIX_PADDING
                + d2_target::PREFIX_WIDTH
                + max_width
                + d2_target::CENTER_PADDING
                + d2_target::TYPE_PADDING;
            let row_count = class_ref_opt
                .as_ref()
                .map(|c| c.fields.len() + c.methods.len())
                .unwrap_or(0) as i32;
            // Go has two separate height formulas depending on whether there
            // are any row texts to measure.
            let h = if row_h > 0 {
                let row_height = row_h + d2_target::VERTICAL_PADDING;
                // label::PADDING = 5 (d2-label crate).
                let header_reserve = (2 * row_height).max(padded_header_h + 2 * 5);
                row_height * row_count + header_reserve
            } else {
                // No fields/methods — Go: `2*max(12, labelDims.Height) + VerticalPadding`
                2 * 12i32.max(padded_header_h) + d2_target::VERTICAL_PADDING
            };

            g.objects[i].width = if desired_width > 0 {
                desired_width as f64
            } else {
                w as f64
            };
            g.objects[i].height = if desired_height > 0 {
                desired_height as f64
            } else {
                h as f64
            };
            g.objects[i].update_box();
            continue;
        }

        // SQL table shapes. Mirrors Go `GetDefaultSize` sql_table branch
        // plus the `withLabelPadding` adjustment at the top of the
        // function that grows labelDims by INNER_LABEL_PADDING when no
        // explicit width/height was requested.
        if shape == "sql_table" {
            let table_font_size = if let Some(v) = g.objects[i].style.font_size.as_ref() {
                v.value.parse().unwrap_or(d2_fonts::FONT_SIZE_L)
            } else {
                d2_fonts::FONT_SIZE_L
            };
            // Header label is measured in the regular (non-mono) font
            // for sql_table — Go uses `GetTextDimensions` in that branch.
            // The font style follows Go `obj.Text()`:
            //   isBold = !IsContainer() && shape != "text"
            //   if OuterSequenceDiagram() != nil { isBold = false }
            // After compilation, sql_table children are moved to columns
            // (is_container = false), so normally isBold = true. But inside
            // a sequence diagram, isBold is forced to false.
            let header_font_size = table_font_size + d2_target::HEADER_FONT_ADD;
            let header_style = if in_seq {
                FontStyle::Regular
            } else if is_bold {
                FontStyle::Bold
            } else {
                FontStyle::Regular
            };
            let header_font = d2_fonts::Font::new(font_family, header_style, header_font_size);
            // Empty-label fallback uses placeholder text "Table" (mirrors
            // Go `GetLabelSize` special case). Go stores the placeholder
            // dimensions on `obj.LabelDimensions` regardless of whether the
            // label is empty — preserving that behaviour keeps the diagram
            // hash identical on empty-labelled sql_tables.
            let header_text: &str = if label.is_empty() { "Table" } else { &label };
            let (raw_header_w, raw_header_h) = ruler.measure(header_font, header_text);
            g.objects[i].label_dimensions = d2_graph::Dimensions {
                width: raw_header_w,
                height: raw_header_h,
            };

            // Apply INNER_LABEL_PADDING when no explicit dims were set
            // (equivalent to Go's `withLabelPadding == true`).
            let with_label_padding = desired_width == 0 && desired_height == 0;
            let pad = if with_label_padding {
                INNER_LABEL_PADDING as i32
            } else {
                0
            };
            let header_w = raw_header_w + pad;
            let header_h = raw_header_h + pad;

            // Columns: for each column, measure name / type / constraint
            // with the regular (non-mono) font at `table_font_size`.
            let col_font = d2_fonts::Font::new(font_family, FontStyle::Regular, table_font_size);
            let mut longest_name_w = 0i32;
            let mut longest_type_w = 0i32;
            let mut longest_constraint_w = 0i32;

            let mut table = g.objects[i].sql_table.clone().unwrap_or_default();
            for col in &mut table.columns {
                let (nw, nh) = ruler.measure(col_font, &col.name.label);
                col.name.label_width = nw;
                col.name.label_height = nh;
                longest_name_w = longest_name_w.max(nw);
                let (tw, th) = ruler.measure(col_font, &col.type_.label);
                col.type_.label_width = tw;
                col.type_.label_height = th;
                longest_type_w = longest_type_w.max(tw);
                let _ = th;
                if !col.constraint.is_empty() {
                    let cstr = col.constraint_abbr();
                    let (cw, _) = ruler.measure(col_font, &cstr);
                    longest_constraint_w = longest_constraint_w.max(cw);
                }
            }
            g.objects[i].sql_table = Some(table);

            // Width = max(12, max(hdrW, rowsW)) where:
            //   hdrW  = HeaderPadding + paddedHeaderW + HeaderPadding
            //   rowsW = NamePadding + maxName + TypePadding + maxType
            //         + TypePadding + maxConstraint + (ConstraintPadding if maxConstraint > 0)
            let header_width = 2 * d2_target::HEADER_PADDING + header_w;
            let mut rows_width = d2_target::NAME_PADDING
                + longest_name_w
                + d2_target::TYPE_PADDING
                + longest_type_w
                + d2_target::TYPE_PADDING
                + longest_constraint_w;
            if longest_constraint_w != 0 {
                rows_width += d2_target::CONSTRAINT_PADDING;
            }
            let w = 12.max(header_width.max(rows_width));

            // Height = max(12, paddedHeaderH * (nCols + 1))
            let row_count = g.objects[i]
                .sql_table
                .as_ref()
                .map(|t| t.columns.len())
                .unwrap_or(0) as i32;
            let h = 12.max(header_h * (row_count + 1));

            g.objects[i].width = if desired_width > 0 {
                desired_width as f64
            } else {
                w as f64
            };
            g.objects[i].height = if desired_height > 0 {
                desired_height as f64
            } else {
                h as f64
            };
            g.objects[i].update_box();
            continue;
        }

        // Image shapes have a fixed default size in Go d2 (128×128 from
        // GetDefaultSize) regardless of label. Apply that *before* the
        // empty-label fast path so a labeled image still gets 128×128.
        if shape == "image" {
            let w_def = if desired_width > 0 {
                desired_width as f64
            } else {
                128.0
            };
            let h_def = if desired_height > 0 {
                desired_height as f64
            } else {
                128.0
            };
            // Still measure the label so SVG can render it next to the icon.
            if !label.is_empty() {
                let (tw, th) = measure_label(
                    ruler,
                    &shape,
                    &g.objects[i].language,
                    font_family,
                    font,
                    font_size,
                    &label,
                )?;
                g.objects[i].label_dimensions = d2_graph::Dimensions {
                    width: tw,
                    height: th,
                };
            }
            g.objects[i].width = w_def;
            g.objects[i].height = h_def;
            g.objects[i].update_box();
            continue;
        }

        if label.is_empty() {
            // No label: use default or desired dimensions
            if shape == "circle" || shape == "square" {
                let side = if desired_width > 0 || desired_height > 0 {
                    desired_width.max(desired_height) as f64
                } else {
                    DEFAULT_SHAPE_SIZE
                };
                g.objects[i].width = side;
                g.objects[i].height = side;
            } else {
                g.objects[i].width = if desired_width > 0 {
                    desired_width as f64
                } else {
                    DEFAULT_SHAPE_SIZE
                };
                g.objects[i].height = if desired_height > 0 {
                    desired_height as f64
                } else {
                    DEFAULT_SHAPE_SIZE
                };
            }
            g.objects[i].update_box();
            continue;
        }

        // Measure the label text
        let (tw, th) = measure_label(
            ruler,
            &shape,
            &g.objects[i].language,
            font_family,
            font,
            font_size,
            &label,
        )?;
        g.objects[i].label_dimensions = d2_graph::Dimensions {
            width: tw,
            height: th,
        };
        // Compute "default dimensions" — the content box the shape needs to
        // wrap. Mirrors Go d2graph.GetDefaultSize: labelDims plus
        // INNER_LABEL_PADDING (5) on each axis when there's no explicit
        // width/height and the shape isn't `text`. Code shapes instead get
        // 0.5em padding per side (fontSize on each axis). Width/height are
        // then floored to MIN_SHAPE_SIZE.
        let with_label_padding =
            desired_width == 0 && desired_height == 0 && shape != "text" && !label.is_empty();
        let (label_pad_x, label_pad_y) = if shape == "code" {
            (f64::from(font_size), f64::from(font_size))
        } else if with_label_padding {
            (INNER_LABEL_PADDING, INNER_LABEL_PADDING)
        } else {
            (0.0, 0.0)
        };
        let mut content_w = (tw as f64 + label_pad_x).max(MIN_SHAPE_SIZE);
        let mut content_h = (th as f64 + label_pad_y).max(MIN_SHAPE_SIZE);
        // For `text` shape the content box can fall below MIN_SHAPE_SIZE in
        // Go (it's bumped back up only when needed); we keep that branch
        // simple by always lifting.
        if shape == "text" {
            content_w = (tw as f64).max(MIN_SHAPE_SIZE);
            content_h = (th as f64).max(MIN_SHAPE_SIZE);
        }

        // Build a Shape wrapper at the content size and ask it to fit. This
        // is the shape-specific path Go calls in `SetDimensions` →
        // `SizeToContent`. The dummy box passed to `Shape::new` must have
        // the *content* size because some shapes (oval especially) use it
        // when computing the fitted dimensions. Note: d2-shape uses
        // PascalCase shape type names (e.g. "Oval"), while the DSL uses
        // lowercase ("oval"); convert via `dsl_shape_to_shape_type`.
        let shape_type_name = d2_target::dsl_shape_to_shape_type(&shape);
        let content_box = d2_geo::Box2D::new(d2_geo::Point::new(0.0, 0.0), content_w, content_h);
        let s = d2_shape::Shape::new(shape_type_name, content_box);
        let (mut pad_x, mut pad_y) = d2_shape::ShapeOps::get_default_padding(&s);
        if desired_width != 0 {
            pad_x = 0.0;
        }
        if desired_height != 0 {
            pad_y = 0.0;
        }

        // Match Go d2graph.SetDimensions: non-image shapes with icons get
        // extra room so the label can sit above/beside the icon cleanly.
        if g.objects[i].icon.is_some() {
            match shape.as_str() {
                "sql_table" | "class" | "code" | "text" => {}
                _ => {
                    let label_height =
                        g.objects[i].label_dimensions.height as f64 + INNER_LABEL_PADDING;
                    if desired_width == 0 {
                        pad_x += label_height;
                    }
                    if desired_height == 0 {
                        pad_y += label_height;
                    }
                }
            }
        }

        // Go reserves extra horizontal room for the link/tooltip affordances.
        if desired_width == 0 && g.objects[i].link.is_some() && g.objects[i].tooltip.is_some() {
            match shape.as_str() {
                "sql_table" | "class" | "code" => {}
                _ => {
                    pad_x += 64.0;
                }
            }
        }

        // Person shapes don't use the per-shape AR/wedge math in
        // get_dimensions_to_fit — Go's SizeToContent special-cases them
        // with `fitWidth = contentWidth + paddingX`. Mirror that.
        let (fit_w, fit_h) = if shape == "person" || shape == "c4_person" {
            (content_w + pad_x, content_h + pad_y)
        } else {
            d2_shape::ShapeOps::get_dimensions_to_fit(&s, content_w, content_h, pad_x, pad_y)
        };

        // SizeToContent: an explicit desired width/height *overrides* the
        // fit, except for class/sql_table/code which take the max.
        let mut w = if desired_width > 0 {
            desired_width as f64
        } else {
            fit_w
        };
        let mut h = if desired_height > 0 {
            desired_height as f64
        } else {
            fit_h
        };
        if g.objects[i].sql_table.is_some()
            || g.objects[i].class.is_some()
            || !g.objects[i].language.is_empty()
        {
            w = (desired_width as f64).max(fit_w);
            h = (desired_height as f64).max(fit_h);
        }

        // Aspect-ratio-1 shapes (RealSquare, Circle) must be square.
        // Person and Oval get an aspect-ratio limit applied next.
        if d2_shape::ShapeOps::aspect_ratio_1(&s) {
            let side = w.max(h);
            w = side;
            h = side;
        } else if desired_height == 0 || desired_width == 0 {
            match shape.as_str() {
                "person" => {
                    let (lw, lh) = d2_shape::limit_ar(w, h, 1.5);
                    w = lw;
                    h = lh;
                }
                "oval" => {
                    let (lw, lh) = d2_shape::limit_ar(w, h, 3.0);
                    w = lw;
                    h = lh;
                }
                _ => {}
            }
        }

        g.objects[i].width = w;
        g.objects[i].height = h;

        // Cloud shapes store the content aspect ratio so the renderer
        // can size the inner content box (Go `SizeToContent` tail).
        if shape == "cloud" {
            if let Some(inner) =
                d2_shape::ShapeOps::get_inner_box_for_content(&s, content_w, content_h)
            {
                if inner.height > 0.0 {
                    g.objects[i].content_aspect_ratio = Some(inner.width / inner.height);
                }
            }
        }

        g.objects[i].update_box();
    }

    // Process edges: measure edge labels
    let edge_count = g.edges.len();
    for i in 0..edge_count {
        g.edges[i].label.value =
            apply_text_transform(&g.edges[i].label.value, &g.edges[i].style, caps_lock, false);
        let label = g.edges[i].label.value.clone();
        let src_ah_label = g.edges[i]
            .src_arrowhead
            .as_ref()
            .map(|ah| ah.label.value.clone())
            .unwrap_or_default();
        let dst_ah_label = g.edges[i]
            .dst_arrowhead
            .as_ref()
            .map(|ah| ah.label.value.clone())
            .unwrap_or_default();

        if label.is_empty() && src_ah_label.is_empty() && dst_ah_label.is_empty() {
            continue;
        }

        let is_bold = g.edges[i]
            .style
            .bold
            .as_ref()
            .is_some_and(|v| v.value == "true");
        // Match Go d2graph.Edge.Text(): edge labels default to italic.
        // An explicit `style.italic: false` can turn it off, but absent
        // a style we still measure with the italic font.
        let is_italic = g.edges[i]
            .style
            .italic
            .as_ref()
            .map_or(true, |v| v.value == "true");
        let font_size: i32 = g.edges[i]
            .style
            .font_size
            .as_ref()
            .and_then(|v| v.value.parse().ok())
            .unwrap_or(FONT_SIZE_M);

        let font_style = if is_bold {
            FontStyle::Bold
        } else if is_italic {
            FontStyle::Italic
        } else {
            FontStyle::Regular
        };

        // Per-edge font override (matches Go d2graph.Edge.Text + GetLabelSize).
        let edge_font_family = match g.edges[i].style.font.as_ref().map(|v| v.value.as_str()) {
            Some("mono") => FontFamily::SourceCodePro,
            _ => default_family,
        };

        let font = d2_fonts::Font::new(edge_font_family, font_style, font_size);
        if !label.is_empty() {
            // Go's edge label measurement follows the same path as
            // GetTextDimensions/GetTextDimensionsWithMono:
            // - If language != "": use MeasureMono with SourceCodePro at
            //   CODE_LINE_HEIGHT (same path as code shape labels)
            // - If language == "markdown": use markdown measurement
            // - Otherwise: regular text measurement with font style
            let edge_language = &g.edges[i].language;
            let (tw, th) = if edge_language == "latex" {
                d2_latex::measure(&label).unwrap_or_else(|_| ruler.measure(font, &label))
            } else if edge_language == "markdown" {
                d2_textmeasure::measure_markdown(
                    &label,
                    ruler,
                    Some(edge_font_family),
                    Some(FontFamily::SourceCodePro),
                    font_size,
                )?
            } else if !edge_language.is_empty() {
                // Non-empty language: Go's GetTextDimensions uses
                // MeasureMono with SourceCodePro Regular + CODE_LINE_HEIGHT
                let original_lh = ruler.line_height_factor;
                ruler.line_height_factor = d2_textmeasure::CODE_LINE_HEIGHT;
                let mono_font = d2_fonts::Font::new(
                    FontFamily::SourceCodePro,
                    d2_fonts::FontStyle::Regular,
                    font_size,
                );
                let (w, mut h) = ruler.measure_mono(mono_font, &label);
                ruler.line_height_factor = original_lh;

                // Count empty leading/trailing lines (same as object code)
                let lines: Vec<&str> = label.split('\n').collect();
                let has_leading = !lines.is_empty()
                    && lines.first().map(|l| l.trim().is_empty()).unwrap_or(false);
                let mut num_trailing = 0usize;
                for l in lines.iter().rev() {
                    if l.trim().is_empty() {
                        num_trailing += 1;
                    } else {
                        break;
                    }
                }
                if has_leading && num_trailing < lines.len() {
                    h += font_size;
                }
                h += (d2_textmeasure::CODE_LINE_HEIGHT * f64::from(font_size * num_trailing as i32))
                    .ceil() as i32;
                (w, h)
            } else {
                // Regular text measurement
                ruler.measure(font, &label)
            };
            g.edges[i].label_dimensions = d2_graph::Dimensions {
                width: tw,
                height: th,
            };
        }
        // Arrowhead labels use the same font as the edge label. Mirrors
        // the block in Go d2graph.SetDimensions that runs before the
        // edge-label branch.
        if !src_ah_label.is_empty() {
            let (tw, th) = ruler.measure(font, &src_ah_label);
            if let Some(ref mut ah) = g.edges[i].src_arrowhead {
                ah.label_dimensions = d2_graph::Dimensions {
                    width: tw,
                    height: th,
                };
            }
        }
        if !dst_ah_label.is_empty() {
            let (tw, th) = ruler.measure(font, &dst_ah_label);
            if let Some(ref mut ah) = g.edges[i].dst_arrowhead {
                ah.label_dimensions = d2_graph::Dimensions {
                    width: tw,
                    height: th,
                };
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

    #[test]
    fn e2e_simple_edge() {
        let svg = d2_to_svg("a -> b").unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(
            svg_str.contains("<svg"),
            "SVG should contain opening <svg tag"
        );
        assert!(
            svg_str.contains("</svg>"),
            "SVG should contain closing </svg> tag"
        );
        // The SVG should contain text elements for "a" and "b"
        assert!(svg_str.contains(">a<"), "SVG should contain label 'a'");
        assert!(svg_str.contains(">b<"), "SVG should contain label 'b'");
    }

    #[test]
    fn e2e_single_node() {
        let svg = d2_to_svg("hello").unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(svg_str.contains("<svg"));
        assert!(svg_str.contains(">hello<"));
    }

    #[test]
    fn e2e_styled_node() {
        let svg = d2_to_svg("x: { style.fill: red }").unwrap();
        assert!(!svg.is_empty());
    }

    #[test]
    fn e2e_edge_chain() {
        let svg = d2_to_svg("a -> b -> c").unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(svg_str.contains("<svg"));
        assert!(svg_str.contains(">a<"));
        assert!(svg_str.contains(">b<"));
        assert!(svg_str.contains(">c<"));
    }

    #[test]
    fn e2e_labeled_edge() {
        let svg = d2_to_svg("a -> b: connects").unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(svg_str.contains("<svg"));
        assert!(svg_str.contains(">a<"));
        assert!(svg_str.contains(">b<"));
    }

    #[test]
    fn e2e_nested_objects() {
        let svg = d2_to_svg("a: {\n  b\n}").unwrap();
        let svg_str = String::from_utf8(svg).unwrap();
        assert!(svg_str.contains("<svg"));
    }

    #[test]
    fn e2e_compile_returns_diagram() {
        let opts = CompileOptions::default();
        let (diagram, svg) = compile("x -> y", &opts).unwrap();
        assert!(!svg.is_empty());
        assert!(!diagram.shapes.is_empty());
        assert!(!diagram.connections.is_empty());
    }

    #[test]
    fn parse_returns_ast() {
        let ast = parse("a -> b").unwrap();
        // The AST should have nodes/edges
        assert!(!ast.nodes.is_empty());
    }
}
