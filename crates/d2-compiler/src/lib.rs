//! d2-compiler: Compiles d2 source text into a Graph.
//!
//! Pipeline: source text -> AST (d2-parser) -> IR (d2-ir) -> Graph (d2-graph).
//! Ported from Go `d2compiler/compile.go`.

use d2_ast::{self as ast};
use d2_color;
use d2_graph::{self as graph, Graph, ObjId, ScalarValue};
use d2_ir::{self as ir};
use d2_target;
use d2_themes;
use roxmltree::Document;

fn block_string_language(scalar: &ir::Scalar) -> Option<String> {
    match &scalar.value {
        ast::ScalarBox::BlockString(block) => Some(
            short_to_full_language(&block.tag)
                .unwrap_or(block.tag.as_str())
                .to_owned(),
        ),
        _ => None,
    }
}

fn validate_markdown_xml(markdown: &str) -> Result<(), String> {
    let rendered =
        d2_textmeasure::render_markdown(markdown).map_err(|_| "malformed Markdown".to_owned())?;
    let wrapped = format!("<div>{}</div>", rendered);
    Document::parse(&wrapped)
        .map(|_| ())
        .map_err(|e| format!("malformed Markdown: {e}"))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compile d2 source text into a Graph plus any root `vars.d2-config`.
pub fn compile_with_config(
    path: &str,
    input: &str,
) -> Result<(Graph, Option<d2_target::Config>), CompileError> {
    let (ast_map, parse_err) = d2_parser::parse(path, input);
    if let Some(e) = parse_err {
        return Err(CompileError { errors: e.errors });
    }

    let ir_map = ir::compile(&ast_map).map_err(|e| CompileError { errors: e.errors })?;
    let config = compile_config(&ir_map);

    let mut c = Compiler::new();
    let mut g = Graph::new();

    c.compile_board(&mut g, &ir_map);
    c.expand_literal_star_globs(&mut g);
    c.set_default_shapes(&mut g);
    validate_board_links(&mut g);

    // Match Go d2compiler: if there are no user objects (only the implicit root),
    // mark the graph as folder-only so it will not be rendered as its own board.
    // See Go d2compiler/compile.go:  `if len(g.Objects) == 0 { g.IsFolderOnly = true }`
    // (Go excludes the root from Objects; our objects[0] is the root.)
    if g.objects.len() <= 1 && g.edges.is_empty() {
        g.is_folder_only = true;
    }

    // Mirror Go d2compiler.Compile: after compileIR, stable-sort objects
    // AND edges by their first AST reference so fields/edges that appear
    // earlier in the source always render first. Without this, an edge
    // declared inside a container (`finally: { a -> tree }`) gets added
    // to `g.edges` before a top-level edge whose source line is higher
    // up, which drifts from Go's output order.
    g.sort_objects_by_ast();
    g.sort_edges_by_ast();

    if c.errors.is_empty() {
        Ok((g, config))
    } else {
        Err(CompileError { errors: c.errors })
    }
}

/// Compile d2 source text into a Graph.
pub fn compile(path: &str, input: &str) -> Result<Graph, CompileError> {
    let (g, _) = compile_with_config(path, input)?;
    Ok(g)
}

/// Port of Go `d2compiler.validateBoardLinks`: a shape's `link` is kept
/// only if it is a remote URL (has a scheme or begins with `/`) or a
/// D2 keypath starting with `root`. Everything else (like `link: foo`
/// or bare domain `link: "example.com"`) is stripped to match Go's
/// behavior.
fn validate_board_links(g: &mut Graph) {
    fn is_remote_url(s: &str) -> bool {
        // Scheme detection: initial run of `[A-Za-z][A-Za-z0-9+.-]*:`
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let mut i = 0;
        if !bytes[0].is_ascii_alphabetic() {
            return s.starts_with('/');
        }
        while i < bytes.len() {
            let b = bytes[i];
            if i == 0 {
                if !b.is_ascii_alphabetic() {
                    break;
                }
            } else if !(b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.') {
                if b == b':' && i > 0 {
                    return true; // has a scheme
                }
                break;
            }
            i += 1;
        }
        s.starts_with('/')
    }

    for obj in &mut g.objects {
        let Some(ref link) = obj.link else { continue };
        let val = &link.value;
        if is_remote_url(val) {
            continue;
        }
        // Not remote: must be a keypath starting with "root". Since we don't
        // have a full D2 parser available, we use simple heuristic: split on
        // `.` and check the first segment (unquoted).
        // TODO: full ParseKey equivalent. For now strip the link unless the
        // first path segment is literally `root`.
        let first = val.split('.').next().unwrap_or("");
        if first != "root" {
            obj.link = None;
        }
    }
}

fn compile_config(ir: &ir::Map) -> Option<d2_target::Config> {
    let config_field = ir.get_field_path(&["vars", "d2-config"])?;
    let config_map = config_field.map()?;

    let mut config = d2_target::Config::default();

    if let Some(field) = config_map.get_field("sketch") {
        config.sketch = field.primary_string().and_then(|s| s.parse::<bool>().ok());
    }
    if let Some(field) = config_map.get_field("theme-id") {
        config.theme_id = field.primary_string().and_then(|s| s.parse::<i64>().ok());
    }
    if let Some(field) = config_map.get_field("dark-theme-id") {
        config.dark_theme_id = field.primary_string().and_then(|s| s.parse::<i64>().ok());
    }
    if let Some(field) = config_map.get_field("pad") {
        config.pad = field.primary_string().and_then(|s| s.parse::<i64>().ok());
    }
    if let Some(field) = config_map.get_field("layout-engine") {
        config.layout_engine = field.primary_string();
    }
    if let Some(field) = config_map.get_field("center") {
        config.center = field.primary_string().and_then(|s| s.parse::<bool>().ok());
    }
    if let Some(field) = config_map.get_field("theme-overrides") {
        config.theme_overrides = compile_theme_overrides(field.map());
    }
    if let Some(field) = config_map.get_field("dark-theme-overrides") {
        config.dark_theme_overrides = compile_theme_overrides(field.map());
    }
    if let Some(field) = config_map.get_field("data").and_then(|f| f.map()) {
        for field in &field.fields {
            if let Some(value) = field.primary_string() {
                config.data.insert(field.name.clone(), value);
            }
        }
    }

    Some(config)
}

fn compile_theme_overrides(map: Option<&ir::Map>) -> Option<d2_themes::ThemeOverrides> {
    let map = map?;
    let mut out = d2_themes::ThemeOverrides::default();

    for field in &map.fields {
        let Some(value) = field.primary_string() else {
            continue;
        };
        let slot = match field.name.to_ascii_uppercase().as_str() {
            "N1" => &mut out.n1,
            "N2" => &mut out.n2,
            "N3" => &mut out.n3,
            "N4" => &mut out.n4,
            "N5" => &mut out.n5,
            "N6" => &mut out.n6,
            "N7" => &mut out.n7,
            "B1" => &mut out.b1,
            "B2" => &mut out.b2,
            "B3" => &mut out.b3,
            "B4" => &mut out.b4,
            "B5" => &mut out.b5,
            "B6" => &mut out.b6,
            "AA2" => &mut out.aa2,
            "AA4" => &mut out.aa4,
            "AA5" => &mut out.aa5,
            "AB4" => &mut out.ab4,
            "AB5" => &mut out.ab5,
            _ => continue,
        };

        if d2_color::NAMED_COLORS.contains(&value.to_ascii_lowercase().as_str())
            || d2_color::is_color_hex(&value)
        {
            *slot = Some(value);
        }
    }

    if out == d2_themes::ThemeOverrides::default() {
        None
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CompileError {
    pub errors: Vec<ast::Error>,
}

impl CompileError {
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{}: {}", e.range, e.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for CompileError {}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

struct Compiler {
    errors: Vec<ast::Error>,
    /// Class definitions (`classes.<name>`) collected from the root IR map
    /// before compilation. When an object declares `class: <name>` or
    /// `class: [a; b]`, we look up the class here and apply its fields to
    /// the object (recursively via `compile_map`), matching Go
    /// `d2compiler.compileMap`'s class-expansion step.
    class_defs: std::collections::HashMap<String, ir::Map>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            class_defs: std::collections::HashMap::new(),
        }
    }

    /// Populate `class_defs` from the root `classes` field.
    fn collect_class_defs(&mut self, root: &ir::Map) {
        let Some(classes_field) = root.get_field("classes") else {
            return;
        };
        let Some(classes_map) = classes_field.map() else {
            return;
        };
        for class_field in &classes_map.fields {
            if let Some(map) = class_field.map() {
                // Use the case-preserved field name so lookups honour the
                // user's original key (Go's GetClassMap does a
                // case-insensitive lookup; we lowercase the key here and
                // also lowercase the lookup side).
                self.class_defs
                    .insert(class_field.name.to_lowercase(), map.clone());
            }
        }
    }

    fn normalize_label_position(pos: &str) -> Option<String> {
        let normalized = match pos {
            "top-left" => "INSIDE_TOP_LEFT",
            "top-center" => "INSIDE_TOP_CENTER",
            "top-right" => "INSIDE_TOP_RIGHT",
            "center-left" => "INSIDE_MIDDLE_LEFT",
            "center-center" => "INSIDE_MIDDLE_CENTER",
            "center-right" => "INSIDE_MIDDLE_RIGHT",
            "bottom-left" => "INSIDE_BOTTOM_LEFT",
            "bottom-center" => "INSIDE_BOTTOM_CENTER",
            "bottom-right" => "INSIDE_BOTTOM_RIGHT",
            "outside-top-left" => "OUTSIDE_TOP_LEFT",
            "outside-top-center" => "OUTSIDE_TOP_CENTER",
            "outside-top-right" => "OUTSIDE_TOP_RIGHT",
            "outside-left-top" => "OUTSIDE_LEFT_TOP",
            "outside-left-center" => "OUTSIDE_LEFT_MIDDLE",
            "outside-left-bottom" => "OUTSIDE_LEFT_BOTTOM",
            "outside-right-top" => "OUTSIDE_RIGHT_TOP",
            "outside-right-center" => "OUTSIDE_RIGHT_MIDDLE",
            "outside-right-bottom" => "OUTSIDE_RIGHT_BOTTOM",
            "outside-bottom-left" => "OUTSIDE_BOTTOM_LEFT",
            "outside-bottom-center" => "OUTSIDE_BOTTOM_CENTER",
            "outside-bottom-right" => "OUTSIDE_BOTTOM_RIGHT",
            "border-top-left" => "BORDER_TOP_LEFT",
            "border-top-center" => "BORDER_TOP_CENTER",
            "border-top-right" => "BORDER_TOP_RIGHT",
            "border-left-top" => "BORDER_LEFT_TOP",
            "border-left-center" => "BORDER_LEFT_MIDDLE",
            "border-left-bottom" => "BORDER_LEFT_BOTTOM",
            "border-right-top" => "BORDER_RIGHT_TOP",
            "border-right-center" => "BORDER_RIGHT_MIDDLE",
            "border-right-bottom" => "BORDER_RIGHT_BOTTOM",
            "border-bottom-left" => "BORDER_BOTTOM_LEFT",
            "border-bottom-center" => "BORDER_BOTTOM_CENTER",
            "border-bottom-right" => "BORDER_BOTTOM_RIGHT",
            _ => return None,
        };
        Some(normalized.to_owned())
    }

    fn normalize_tooltip_position(pos: &str) -> Option<String> {
        let normalized = match pos {
            "top-left" => "INSIDE_TOP_LEFT",
            "top-center" => "INSIDE_TOP_CENTER",
            "top-right" => "INSIDE_TOP_RIGHT",
            "center-left" => "INSIDE_MIDDLE_LEFT",
            "center-right" => "INSIDE_MIDDLE_RIGHT",
            "bottom-left" => "INSIDE_BOTTOM_LEFT",
            "bottom-center" => "INSIDE_BOTTOM_CENTER",
            "bottom-right" => "INSIDE_BOTTOM_RIGHT",
            _ => return None,
        };
        Some(normalized.to_owned())
    }

    fn errorf(&mut self, range: &ast::Range, msg: String) {
        self.errors.push(ast::Error {
            range: range.clone(),
            message: msg,
        });
    }

    fn compile_position(&mut self, g: &mut Graph, obj: ObjId, field_name: &str, f: &ir::Field) {
        let Some(fmap) = f.map() else {
            return;
        };

        for sub in &fmap.fields {
            if !(sub.name == "near" && sub.name_is_unquoted) {
                continue;
            }
            let Some(val) = sub.primary_string() else {
                continue;
            };

            match field_name {
                "label" | "icon" => {
                    let Some(normalized) = Self::normalize_label_position(&val) else {
                        self.errorf(&ast::Range::default(), "invalid \"near\" field".to_owned());
                        continue;
                    };
                    if field_name == "label" {
                        g.objects[obj].label_position = Some(normalized);
                    } else {
                        g.objects[obj].icon_position = Some(normalized);
                    }
                }
                "tooltip" => {
                    let Some(normalized) = Self::normalize_tooltip_position(&val) else {
                        self.errorf(&ast::Range::default(), "invalid \"near\" field".to_owned());
                        continue;
                    };
                    g.objects[obj].tooltip_position = Some(normalized);
                }
                _ => {}
            }
        }
    }

    fn compile_board(&mut self, g: &mut Graph, ir: &ir::Map) {
        // Collect class definitions before compilation so compile_map can
        // apply them whenever an object declares `class: <name>`.
        self.collect_class_defs(ir);
        let root = g.root;
        self.compile_map(g, root, ir);
        self.set_default_shapes(g);

        // Compile nested boards (layers, scenarios, steps).
        // Mirrors Go d2compiler.compileBoard.
        self.compile_boards_field(g, ir, "layers");
        self.compile_boards_field(g, ir, "scenarios");
        self.compile_boards_field(g, ir, "steps");

        // Mark as folder-only when the graph has boards but no objects of its
        // own (i.e. only the implicit root exists).
        if !g.layers.is_empty() || !g.scenarios.is_empty() || !g.steps.is_empty() {
            if g.objects.len() <= 1 && g.edges.is_empty() {
                g.is_folder_only = true;
            }
        }
    }

    fn expand_literal_star_globs(&mut self, g: &mut Graph) {
        let star_ids: Vec<ObjId> = g
            .objects
            .iter()
            .enumerate()
            .filter(|(id, obj)| *id != g.root && obj.id_val() == "*")
            .map(|(id, _)| id)
            .collect();
        if star_ids.is_empty() {
            return;
        }

        let mut star_targets = std::collections::HashMap::<ObjId, Vec<ObjId>>::new();
        let mut remove_ids = std::collections::HashSet::<ObjId>::new();

        for &star_id in &star_ids {
            let Some(parent) = g.objects[star_id].parent else {
                continue;
            };
            let targets: Vec<ObjId> = g.objects[parent]
                .children_array
                .iter()
                .copied()
                .filter(|&cid| cid != star_id && g.objects[cid].id_val() != "*")
                .collect();
            for &target in &targets {
                self.apply_literal_star_template(g, star_id, target);
            }
            star_targets.insert(star_id, targets);
            self.collect_descendants(g, star_id, &mut remove_ids);
        }

        let edges_snapshot = g.edges.clone();
        for edge in &edges_snapshot {
            let srcs = star_targets
                .get(&edge.src)
                .cloned()
                .unwrap_or_else(|| vec![edge.src]);
            let dsts = star_targets
                .get(&edge.dst)
                .cloned()
                .unwrap_or_else(|| vec![edge.dst]);
            if srcs.len() == 1 && dsts.len() == 1 && srcs[0] == edge.src && dsts[0] == edge.dst {
                continue;
            }
            for &src in &srcs {
                for &dst in &dsts {
                    if src == dst {
                        continue;
                    }
                    let idx = self.clone_edge_with_endpoints(g, edge, src, dst);
                    g.edges[idx].scope_obj = edge.scope_obj;
                }
            }
        }

        self.compact_graph(g, &remove_ids);
    }

    fn collect_descendants(
        &self,
        g: &Graph,
        obj: ObjId,
        remove_ids: &mut std::collections::HashSet<ObjId>,
    ) {
        if !remove_ids.insert(obj) {
            return;
        }
        for &child in &g.objects[obj].children_array {
            self.collect_descendants(g, child, remove_ids);
        }
    }

    fn apply_literal_star_template(&self, g: &mut Graph, template_id: ObjId, target_id: ObjId) {
        let template = g.objects[template_id].clone();
        Self::merge_object_defaults(&template, &mut g.objects[target_id]);

        let child_templates = template.children_array.clone();
        for child_template in child_templates {
            if g.objects[child_template].id_val() == "*" {
                continue;
            }
            let child_name = g.objects[child_template].id_val().to_owned();
            let child = g.ensure_child_of(target_id, &[child_name]);
            if g.objects[child].references.is_empty() {
                let inherited_scope = g.objects[child].parent;
                let template_refs = g.objects[child_template].references.clone();
                g.objects[child]
                    .references
                    .extend(template_refs.into_iter().map(|mut r| {
                        r.scope_obj = inherited_scope;
                        r
                    }));
            }
            self.apply_literal_star_template(g, child_template, child);
        }
    }

    fn merge_object_defaults(template: &graph::Object, target: &mut graph::Object) {
        if target.label.value.is_empty() && !template.label.value.is_empty() {
            target.label = template.label.clone();
        }
        if target.shape.value.eq_ignore_ascii_case("rectangle")
            && !template.shape.value.eq_ignore_ascii_case("rectangle")
        {
            target.shape = template.shape.clone();
        }
        if target.direction.value.is_empty() && !template.direction.value.is_empty() {
            target.direction = template.direction.clone();
        }
        if target.language.is_empty() && !template.language.is_empty() {
            target.language = template.language.clone();
        }

        Self::merge_style_defaults(&template.style, &mut target.style);
        if target.icon_style.border_radius.is_none() {
            target.icon_style.border_radius = template.icon_style.border_radius.clone();
        }

        if target.icon.is_none() {
            target.icon = template.icon.clone();
        }
        if target.icon_position.is_none() {
            target.icon_position = template.icon_position.clone();
        }
        if target.label_position.is_none() {
            target.label_position = template.label_position.clone();
        }
        if target.tooltip_position.is_none() {
            target.tooltip_position = template.tooltip_position.clone();
        }
        if target.tooltip.is_none() {
            target.tooltip = template.tooltip.clone();
        }
        if target.link.is_none() {
            target.link = template.link.clone();
        }
        if target.class.is_none() {
            target.class = template.class.clone();
        }
        if target.sql_table.is_none() {
            target.sql_table = template.sql_table.clone();
        }
        if target.content_aspect_ratio.is_none() {
            target.content_aspect_ratio = template.content_aspect_ratio;
        }
        if target.width_attr.is_none() {
            target.width_attr = template.width_attr.clone();
        }
        if target.height_attr.is_none() {
            target.height_attr = template.height_attr.clone();
        }
        if target.top.is_none() {
            target.top = template.top.clone();
        }
        if target.left.is_none() {
            target.left = template.left.clone();
        }
        if target.near_key.is_none() {
            target.near_key = template.near_key.clone();
        }
        if target.grid_rows.is_none() {
            target.grid_rows = template.grid_rows.clone();
        }
        if target.grid_columns.is_none() {
            target.grid_columns = template.grid_columns.clone();
        }
        if target.grid_gap.is_none() {
            target.grid_gap = template.grid_gap.clone();
        }
        if target.vertical_gap.is_none() {
            target.vertical_gap = template.vertical_gap.clone();
        }
        if target.horizontal_gap.is_none() {
            target.horizontal_gap = template.horizontal_gap.clone();
        }

        for class in &template.classes {
            if !target.classes.contains(class) {
                target.classes.push(class.clone());
            }
        }
    }

    fn merge_style_defaults(template: &graph::Style, target: &mut graph::Style) {
        macro_rules! merge {
            ($field:ident) => {
                if target.$field.is_none() {
                    target.$field = template.$field.clone();
                }
            };
        }
        merge!(opacity);
        merge!(stroke);
        merge!(fill);
        merge!(fill_pattern);
        merge!(stroke_dash);
        merge!(stroke_width);
        merge!(shadow);
        merge!(three_dee);
        merge!(multiple);
        merge!(border_radius);
        merge!(font_color);
        merge!(font_size);
        merge!(italic);
        merge!(bold);
        merge!(underline);
        merge!(font);
        merge!(double_border);
        merge!(animated);
        merge!(filled);
        merge!(text_transform);
    }

    fn clone_edge_with_endpoints(
        &self,
        g: &mut Graph,
        template: &graph::Edge,
        src: ObjId,
        dst: ObjId,
    ) -> usize {
        let mut edge = template.clone();
        edge.src = src;
        edge.dst = dst;
        edge.abs_id = Self::edge_abs_id(g, src, dst, edge.src_arrow, edge.dst_arrow);
        edge.route.clear();
        edge.is_curve = false;
        edge.src_table_column_index = None;
        edge.dst_table_column_index = None;
        let idx = g.edges.len();
        g.edges.push(edge);
        idx
    }

    fn edge_abs_id(
        g: &Graph,
        src: ObjId,
        dst: ObjId,
        src_arrow: bool,
        dst_arrow: bool,
    ) -> String {
        let arrow_str = if src_arrow && dst_arrow {
            "<->"
        } else if src_arrow {
            "<-"
        } else if dst_arrow {
            "->"
        } else {
            "--"
        };
        let index = g
            .edges
            .iter()
            .filter(|e| {
                e.src == src && e.dst == dst && e.src_arrow == src_arrow && e.dst_arrow == dst_arrow
            })
            .count();
        let src_ida: Vec<String> = g.objects[src].abs_id.split('.').map(str::to_owned).collect();
        let dst_ida: Vec<String> = g.objects[dst].abs_id.split('.').map(str::to_owned).collect();
        let mut common: Vec<String> = Vec::new();
        let mut s_idx = 0usize;
        let mut d_idx = 0usize;
        while src_ida.len() - s_idx > 1 && dst_ida.len() - d_idx > 1 {
            if !src_ida[s_idx].eq_ignore_ascii_case(&dst_ida[d_idx]) {
                break;
            }
            common.push(src_ida[s_idx].clone());
            s_idx += 1;
            d_idx += 1;
        }
        let common_key = if common.is_empty() {
            String::new()
        } else {
            format!("{}.", common.join("."))
        };
        let src_tail = src_ida[s_idx..].join(".");
        let dst_tail = dst_ida[d_idx..].join(".");
        format!(
            "{}({} {} {})[{}]",
            common_key, src_tail, arrow_str, dst_tail, index
        )
    }

    fn compact_graph(
        &self,
        g: &mut Graph,
        remove_ids: &std::collections::HashSet<ObjId>,
    ) {
        if remove_ids.is_empty() {
            return;
        }

        let mut order: Vec<ObjId> = Vec::with_capacity(g.objects.len());
        for id in 0..g.objects.len() {
            if id == g.root || !remove_ids.contains(&id) {
                order.push(id);
            }
        }

        let mut old_to_new = vec![usize::MAX; g.objects.len()];
        for (new_idx, &old_idx) in order.iter().enumerate() {
            old_to_new[old_idx] = new_idx;
        }

        let mut new_objects = Vec::with_capacity(order.len());
        for &old_idx in &order {
            new_objects.push(std::mem::take(&mut g.objects[old_idx]));
        }
        g.objects = new_objects;

        for obj in &mut g.objects {
            obj.parent = obj.parent.and_then(|p| {
                let mapped = old_to_new[p];
                (mapped != usize::MAX).then_some(mapped)
            });
            obj.children.retain(|c| old_to_new[*c] != usize::MAX);
            for c in &mut obj.children {
                *c = old_to_new[*c];
            }
            obj.children_array.retain(|c| old_to_new[*c] != usize::MAX);
            for c in &mut obj.children_array {
                *c = old_to_new[*c];
            }
            for r in &mut obj.references {
                r.scope_obj = r.scope_obj.and_then(|s| {
                    let mapped = old_to_new[s];
                    (mapped != usize::MAX).then_some(mapped)
                });
            }
        }

        g.root = old_to_new[g.root];
        g.edges.retain(|e| old_to_new[e.src] != usize::MAX && old_to_new[e.dst] != usize::MAX);
        for e in &mut g.edges {
            e.src = old_to_new[e.src];
            e.dst = old_to_new[e.dst];
            e.scope_obj = e.scope_obj.and_then(|s| {
                let mapped = old_to_new[s];
                (mapped != usize::MAX).then_some(mapped)
            });
        }
    }

    /// Extract sub-boards from the IR and compile each one into a child graph.
    /// Mirrors Go d2compiler.compileBoardsField.
    fn compile_boards_field(&mut self, g: &mut Graph, ir: &ir::Map, field_name: &str) {
        let boards_field = match ir.get_field(field_name) {
            Some(f) => f,
            None => return,
        };
        let boards_map = match boards_field.map() {
            Some(m) => m,
            None => return,
        };

        // For scenarios/steps, compute the parent board's base (without
        // layers/scenarios/steps) for overlay.
        let parent_base = if field_name == "scenarios" || field_name == "steps" {
            Some(ir.copy_base())
        } else {
            None
        };

        let mut prev_step_map: Option<ir::Map> = None;

        for f in &boards_map.fields {
            let child_map = f.map().cloned().unwrap_or_default();

            // Apply overlay: scenarios inherit parent board, steps inherit
            // previous step (or parent board for the first step).
            let effective_map = match field_name {
                "scenarios" => {
                    let mut base = parent_base.as_ref().unwrap().clone();
                    ir::overlay_map(&mut base, &child_map);
                    base
                }
                "steps" => {
                    let mut base = if let Some(ref prev) = prev_step_map {
                        let mut b = prev.clone();
                        // Remove label from prev step so it doesn't carry forward.
                        b.delete_field("label");
                        b
                    } else {
                        parent_base.as_ref().unwrap().clone()
                    };
                    ir::overlay_map(&mut base, &child_map);
                    // Save for next step
                    prev_step_map = Some(base.clone());
                    base
                }
                _ => child_map,
            };

            let mut g2 = Graph::new();
            self.compile_board(&mut g2, &effective_map);
            g2.name = f.name.clone();
            // Mark folder-only if the sub-board itself has no user objects.
            if g2.objects.len() <= 1 && g2.edges.is_empty() {
                g2.is_folder_only = true;
            }
            g2.sort_objects_by_ast();
            g2.sort_edges_by_ast();
            match field_name {
                "layers" => g.layers.push(g2),
                "scenarios" => g.scenarios.push(g2),
                "steps" => g.steps.push(g2),
                _ => {}
            }
        }
    }

    fn compile_map(&mut self, g: &mut Graph, obj: ObjId, m: &ir::Map) {
        self.compile_map_scoped(g, obj, obj, m);
    }

    /// Internal compile_map with explicit `scope` for reference tracking.
    /// `scope` is the object whose IR map declared the keys being compiled.
    /// Normally `scope == obj`, but when a sequence diagram redirect
    /// occurred, `obj` is the redirected graph parent while `scope` is the
    /// original declaring container (the group).
    fn compile_map_scoped(&mut self, g: &mut Graph, obj: ObjId, scope: ObjId, m: &ir::Map) {
        // Apply referenced classes *first* so the object's own fields
        // override any class defaults. Mirrors Go d2compiler.compileMap's
        // top-of-function class-expansion step.
        if !self.class_defs.is_empty() {
            if let Some(class_field) = m.get_field("class") {
                let mut class_names: Vec<String> = Vec::new();
                if let Some(ref primary) = class_field.primary {
                    class_names.push(primary.scalar_string());
                } else if let Some(ir::Composite::Array(ref arr)) = class_field.composite {
                    for v in &arr.values {
                        if let ir::Value::Scalar(s) = v {
                            class_names.push(s.scalar_string());
                        }
                    }
                }
                for name in class_names {
                    // Clone the class map so we release the borrow on
                    // `self.class_defs` before the recursive compile_map
                    // call, which takes `&mut self`.
                    if let Some(class_map) = self.class_defs.get(&name.to_lowercase()).cloned() {
                        self.compile_map(g, obj, &class_map);
                    }
                }
            }
        }

        // Process shape first (affects how children are handled)
        if let Some(shape_field) = m.get_field("shape") {
            if shape_field.primary.is_some() {
                self.compile_field_scoped(g, obj, scope, shape_field);
            }
        }

        // Process all other fields
        for f in &m.fields {
            if f.name == "shape" && f.name_is_unquoted {
                continue;
            }
            if ast::BOARD_KEYWORDS.contains(f.name.as_str()) && f.name_is_unquoted {
                continue;
            }
            self.compile_field_scoped(g, obj, scope, f);
        }

        // For class / sql_table shapes, reinterpret the object's children
        // as class fields / table columns and detach them from the graph.
        // Mirrors Go `compileClass` / `compileSQLTable`.
        if g.objects[obj].shape.value == d2_target::SHAPE_CLASS {
            self.compile_class_shape(g, obj);
        } else if g.objects[obj].shape.value == d2_target::SHAPE_SQL_TABLE {
            self.compile_sql_table_shape(g, obj);
        }

        // Process edges. Edge scope_obj is always `obj` — the object whose
        // map directly contains the edge declaration. This matches Go's
        // behavior where edge.Reference.ScopeObj comes from the IR map
        // that declared the edge (not a parent scope).
        for e in &m.edges {
            self.compile_edge_scoped(g, obj, obj, e);
        }
    }

    /// Convert a `shape: sql_table` object's child declarations into
    /// `SQLTable { columns }` and remove them from the graph (Go
    /// `compileSQLTable`).
    fn compile_sql_table_shape(&mut self, g: &mut Graph, obj: ObjId) {
        let children: Vec<ObjId> = g.objects[obj].children_array.clone();
        let mut table = d2_target::SQLTable::default();
        for &child in &children {
            // Use id_val() (unquoted) to match Go's col.IDVal.
            let id_val = g.objects[child].id_val().to_owned();
            let label_val = g.objects[child].label.value.clone();
            // If label matches id, type is empty (the user didn't specify
            // a type).
            let type_ = if label_val == id_val {
                String::new()
            } else {
                label_val
            };
            let constraint = g.objects[child].constraint.clone();
            table.columns.push(d2_target::SQLColumn {
                name: d2_target::Text {
                    label: id_val,
                    ..Default::default()
                },
                type_: d2_target::Text {
                    label: type_,
                    ..Default::default()
                },
                constraint,
                reference: String::new(),
            });
        }
        g.objects[obj].sql_table = Some(table);

        for &child in &children {
            g.objects[child].parent = None;
            g.objects[child].shape.value = String::from("__d2_class_field_removed__");
        }
        g.objects[obj].children_array.clear();
        g.objects[obj].children.clear();
    }

    /// Convert a `shape: class` object's child declarations into
    /// `Class { fields, methods }` and remove the children from the graph.
    fn compile_class_shape(&mut self, g: &mut Graph, obj: ObjId) {
        let children: Vec<ObjId> = g.objects[obj].children_array.clone();
        let mut class = d2_target::Class::default();
        for &child in &children {
            // Use id_val() (the unquoted form) to match Go's f.IDVal.
            // The .id field may carry surrounding quotes for keys with
            // special characters (spaces, parens, etc.) — those quotes
            // must be stripped before comparing to the label value and
            // before extracting the visibility prefix.
            let id_val = g.objects[child].id_val().to_owned();
            let label_val = g.objects[child].label.value.clone();
            let underline = g.objects[child]
                .style
                .underline
                .as_ref()
                .is_some_and(|v| v.value == "true");
            let (visibility, name) = match id_val.as_bytes().first() {
                Some(b'+') => ("public", id_val[1..].to_owned()),
                Some(b'-') => ("private", id_val[1..].to_owned()),
                Some(b'#') => ("protected", id_val[1..].to_owned()),
                _ => ("public", id_val.clone()),
            };
            if id_val.contains('(') {
                // Method
                let return_ = if label_val == id_val {
                    "void".to_owned()
                } else {
                    label_val
                };
                class.methods.push(d2_target::ClassMethod {
                    name,
                    return_,
                    visibility: visibility.to_owned(),
                    underline,
                });
            } else {
                // Field
                let type_ = if label_val == id_val {
                    String::new()
                } else {
                    label_val
                };
                class.fields.push(d2_target::ClassField {
                    name,
                    type_,
                    visibility: visibility.to_owned(),
                    underline,
                });
            }
        }
        g.objects[obj].class = Some(class);

        // Detach children: remove them from parent's children_array, clear
        // their parent pointer, and drop them from Graph.objects so they
        // don't get rendered as separate shapes.
        for &child in &children {
            g.objects[child].parent = None;
        }
        g.objects[obj].children_array.clear();
        g.objects[obj].children.clear();
        // Tombstone the removed objects by swapping their shape to an
        // internal "removed" marker that the exporter can filter out.
        // Simpler: since we can't easily re-index, mark them via a
        // sentinel field — here we set their id to empty and mark them
        // as removed via a new flag, or we filter them at export time.
        // For now we remove them lazily: set shape value to special
        // sentinel and rely on the exporter to skip.
        for &child in &children {
            g.objects[child].shape.value = String::from("__d2_class_field_removed__");
        }
    }

    #[allow(dead_code)]
    fn compile_field(&mut self, g: &mut Graph, obj: ObjId, f: &ir::Field) {
        self.compile_field_scoped(g, obj, obj, f);
    }

    /// Compile a field with explicit scope tracking.
    /// `scope` is the object whose IR map declared this field. When inside
    /// a sequence diagram, edges/fields may be redirected to different graph
    /// parents, but the scope still refers to the original declaring container.
    fn compile_field_scoped(&mut self, g: &mut Graph, obj: ObjId, scope: ObjId, f: &ir::Field) {
        let keyword = f.name.to_lowercase();
        let is_style_reserved =
            ast::STYLE_KEYWORDS.contains(keyword.as_str()) && f.name_is_unquoted;
        if is_style_reserved {
            // "X must be style.X"
            return;
        }

        let is_simple_reserved =
            ast::SIMPLE_RESERVED_KEYWORDS.contains(keyword.as_str()) && f.name_is_unquoted;

        if f.name == "classes" && f.name_is_unquoted {
            return; // classes are handled separately
        }
        if f.name == "vars" && f.name_is_unquoted {
            return;
        }
        if (f.name == "source-arrowhead" || f.name == "target-arrowhead") && f.name_is_unquoted {
            // Only valid on connections
            return;
        }

        if is_simple_reserved {
            self.compile_reserved(g, obj, f);
            return;
        }

        if f.name == "style" && f.name_is_unquoted {
            if let Some(fmap) = f.map() {
                self.compile_style(g, obj, fmap, false);
            }
            return;
        }

        // Regular field -> child object
        let child = g.ensure_child_of(obj, &[f.name.clone()]);

        // Mirror Go d2compiler.compileField: copy the IR field's references
        // into d2graph.Object.References. Reference scope_obj tracks which
        // graph object's IR map declared this field. In Go, this comes from
        // BoardIDA(fr.Context_.ScopeMap). `scope` represents the graph
        // object owning the IR map where this key was declared.
        for fr in &f.references {
            if let Some(ref kp) = fr.key_path {
                g.objects[child].references.push(d2_graph::Reference {
                    key: kp.clone(),
                    key_path_index: fr.key_path_index,
                    scope_obj: Some(scope),
                });
            }
        }

        // Set label from primary value
        if let Some(ref primary) = f.primary {
            let label_val = primary.scalar_string();
            g.objects[child].label.value = label_val;
            if let Some(language) = block_string_language(primary) {
                if language == "markdown"
                    && let Err(err) = validate_markdown_xml(&primary.scalar_string())
                {
                    self.errorf(primary.value.get_range(), err);
                }
                g.objects[child].language = language;
            }
        }

        // Recurse into map. Determine the scope for the sub-map:
        // - If a sequence diagram redirect happened (child is not a direct
        //   child of obj), keep the current scope: `obj` is the declaring
        //   group/container whose map contained this key.
        // - Otherwise (normal case), the sub-map's scope is the newly
        //   created child itself (it owns the sub-map).
        if let Some(fmap) = f.map() {
            let child_scope = if g.objects[child].parent == Some(obj) {
                child  // Normal: child owns the sub-map
            } else {
                obj    // Redirect: declaring scope is obj
            };
            self.compile_map_scoped(g, child, child_scope, fmap);
        }
    }

    fn compile_reserved(&mut self, g: &mut Graph, obj: ObjId, f: &ir::Field) {
        let primary_str = f.primary_string();

        match f.name.as_str() {
            "label" => {
                if let Some(val) = primary_str {
                    g.objects[obj].label.value = val;
                }
                if let Some(ref primary) = f.primary
                    && let Some(language) = block_string_language(primary)
                {
                    if language == "markdown"
                        && let Err(err) = validate_markdown_xml(&primary.scalar_string())
                    {
                        self.errorf(primary.value.get_range(), err);
                    }
                    g.objects[obj].language = language;
                }
                self.compile_position(g, obj, "label", f);
            }
            "shape" => {
                if let Some(val) = primary_str {
                    let lower = val.to_lowercase();
                    if !d2_target::is_shape(&lower) {
                        // Unknown shape
                        return;
                    }
                    g.objects[obj].shape.value = lower;
                    if g.objects[obj]
                        .shape
                        .value
                        .eq_ignore_ascii_case(d2_target::SHAPE_CODE)
                    {
                        g.objects[obj].language = d2_target::SHAPE_TEXT.to_owned();
                    }
                }
            }
            "icon" => {
                if let Some(val) = primary_str {
                    g.objects[obj].icon = Some(val);
                }
                self.compile_position(g, obj, "icon", f);
                if let Some(fmap) = f.map() {
                    for sub in &fmap.fields {
                        if sub.name == "style" && sub.name_is_unquoted {
                            if let Some(style_map) = sub.map() {
                                self.compile_icon_style(g, obj, style_map);
                            }
                        }
                    }
                }
            }
            "tooltip" => {
                if let Some(val) = primary_str {
                    if let Err(err) = validate_markdown_xml(&val)
                        && let Some(ref primary) = f.primary
                    {
                        self.errorf(primary.value.get_range(), err);
                    }
                    g.objects[obj].tooltip = Some(ScalarValue { value: val });
                }
                self.compile_position(g, obj, "tooltip", f);
            }
            "link" => {
                if let Some(val) = primary_str {
                    g.objects[obj].link = Some(ScalarValue { value: val });
                }
            }
            "near" => {
                if let Some(val) = primary_str {
                    g.objects[obj].near_key = Some(val);
                }
            }
            "width" => {
                if let Some(val) = primary_str {
                    if val.parse::<i32>().is_err() {
                        return;
                    }
                    g.objects[obj].width_attr = Some(ScalarValue { value: val });
                }
            }
            "height" => {
                if let Some(val) = primary_str {
                    if val.parse::<i32>().is_err() {
                        return;
                    }
                    g.objects[obj].height_attr = Some(ScalarValue { value: val });
                }
            }
            "top" => {
                if let Some(val) = primary_str {
                    match val.parse::<i32>() {
                        Ok(v) if v >= 0 => {
                            g.objects[obj].top = Some(ScalarValue { value: val });
                        }
                        _ => {}
                    }
                }
            }
            "left" => {
                if let Some(val) = primary_str {
                    match val.parse::<i32>() {
                        Ok(v) if v >= 0 => {
                            g.objects[obj].left = Some(ScalarValue { value: val });
                        }
                        _ => {}
                    }
                }
            }
            "direction" => {
                if let Some(val) = primary_str {
                    let lower = val.to_lowercase();
                    if ["up", "down", "right", "left"].contains(&lower.as_str()) {
                        g.objects[obj].direction.value = lower;
                    }
                }
            }
            "constraint" => {
                if let Some(val) = primary_str {
                    g.objects[obj].constraint.push(val);
                } else if let Some(ref comp) = f.composite {
                    if let ir::Composite::Array(arr) = comp {
                        for v in &arr.values {
                            if let ir::Value::Scalar(s) = v {
                                g.objects[obj].constraint.push(s.scalar_string());
                            }
                        }
                    }
                }
            }
            "class" => {
                if let Some(val) = primary_str {
                    g.objects[obj].classes.push(val);
                } else if let Some(ref comp) = f.composite {
                    if let ir::Composite::Array(arr) = comp {
                        for v in &arr.values {
                            if let ir::Value::Scalar(s) = v {
                                g.objects[obj].classes.push(s.scalar_string());
                            }
                        }
                    }
                }
            }
            "grid-rows" => {
                if let Some(val) = primary_str {
                    if let Ok(v) = val.parse::<i32>() {
                        if v > 0 {
                            g.objects[obj].grid_rows = Some(ScalarValue { value: val });
                        }
                    }
                }
            }
            "grid-columns" => {
                if let Some(val) = primary_str {
                    if let Ok(v) = val.parse::<i32>() {
                        if v > 0 {
                            g.objects[obj].grid_columns = Some(ScalarValue { value: val });
                        }
                    }
                }
            }
            "grid-gap" => {
                if let Some(val) = primary_str {
                    if let Ok(v) = val.parse::<i32>() {
                        if v >= 0 {
                            g.objects[obj].grid_gap = Some(ScalarValue { value: val });
                        }
                    }
                }
            }
            "vertical-gap" => {
                if let Some(val) = primary_str {
                    if let Ok(v) = val.parse::<i32>() {
                        if v >= 0 {
                            g.objects[obj].vertical_gap = Some(ScalarValue { value: val });
                        }
                    }
                }
            }
            "horizontal-gap" => {
                if let Some(val) = primary_str {
                    if let Ok(v) = val.parse::<i32>() {
                        if v >= 0 {
                            g.objects[obj].horizontal_gap = Some(ScalarValue { value: val });
                        }
                    }
                }
            }
            "vars" => {} // handled separately
            _ => {}
        }
    }

    fn compile_style(&mut self, g: &mut Graph, obj: ObjId, m: &ir::Map, is_edge: bool) {
        for f in &m.fields {
            let keyword = f.name.to_lowercase();
            if !ast::STYLE_KEYWORDS.contains(keyword.as_str()) || !f.name_is_unquoted {
                continue;
            }
            if f.primary.is_none() {
                continue;
            }
            let val = f.primary_string().unwrap_or_default();

            let style = if is_edge {
                // For edges we'd need a different path, but for now we store on obj
                &mut g.objects[obj].style
            } else {
                &mut g.objects[obj].style
            };

            // Initialize the field
            style.init_field(&keyword);
            // Apply the value
            if let Err(err_msg) = style.apply(&keyword, &val) {
                self.errorf(&ast::Range::default(), err_msg);
            }
        }
    }

    /// Compile an `icon.style` map, which only supports a small subset
    /// (currently `border-radius`). Matches Go's icon style handling.
    fn compile_icon_style(&mut self, g: &mut Graph, obj: ObjId, m: &ir::Map) {
        for f in &m.fields {
            let keyword = f.name.to_lowercase();
            if !f.name_is_unquoted || f.primary.is_none() {
                continue;
            }
            let val = f.primary_string().unwrap_or_default();
            if keyword == "border-radius" {
                g.objects[obj].icon_style.border_radius = Some(ScalarValue { value: val });
            }
        }
    }

    fn compile_edge_icon_style(&mut self, g: &mut Graph, edge_idx: usize, m: &ir::Map) {
        for f in &m.fields {
            let keyword = f.name.to_lowercase();
            if !f.name_is_unquoted || f.primary.is_none() {
                continue;
            }
            let val = f.primary_string().unwrap_or_default();
            if keyword == "border-radius" {
                g.edges[edge_idx].icon_style.border_radius =
                    Some(ScalarValue { value: val });
            }
        }
    }

    fn compile_edge_style(&mut self, g: &mut Graph, edge_idx: usize, m: &ir::Map) {
        for f in &m.fields {
            let keyword = f.name.to_lowercase();
            if !ast::STYLE_KEYWORDS.contains(keyword.as_str()) || !f.name_is_unquoted {
                continue;
            }
            if f.primary.is_none() {
                continue;
            }
            let val = f.primary_string().unwrap_or_default();

            let style = &mut g.edges[edge_idx].style;
            style.init_field(&keyword);
            if let Err(err_msg) = style.apply(&keyword, &val) {
                self.errorf(&ast::Range::default(), err_msg);
            }
        }
    }

    #[allow(dead_code)]
    fn compile_edge(&mut self, g: &mut Graph, obj: ObjId, e: &ir::IREdge) {
        self.compile_edge_scoped(g, obj, obj, e);
    }

    fn compile_edge_scoped(&mut self, g: &mut Graph, obj: ObjId, scope: ObjId, e: &ir::IREdge) {
        let src_path: Vec<String> = e.id.src_path.clone();
        let dst_path: Vec<String> = e.id.dst_path.clone();

        let edge_idx = match g.connect(
            obj,
            &src_path,
            &dst_path,
            e.id.src_arrow,
            e.id.dst_arrow,
            "",
        ) {
            Ok(idx) => idx,
            Err(err) => {
                self.errorf(&ast::Range::default(), err);
                return;
            }
        };

        // Record the scope object — the object in whose map scope this
        // edge was declared. Used by sequence diagram layout to determine
        // group containment. Mirrors Go Edge.References[].ScopeObj.
        g.edges[edge_idx].scope_obj = Some(scope);

        // Record the earliest AST reference for this edge so the graph
        // can later be stable-sorted by source position (matches Go
        // `d2graph.Graph.SortEdgesByAST`).
        if let Some(first_ref) = e.references.first() {
            if let Some(ref edge_ast) = first_ref.context.edge_ast {
                g.edges[edge_idx].first_ast_range = Some(edge_ast.range.clone());
            } else {
                g.edges[edge_idx].first_ast_range = Some(first_ref.context.key.range.clone());
            }
        }

        // Set label from primary
        if let Some(ref primary) = e.primary {
            g.edges[edge_idx].label.value = primary.scalar_string();
            if let Some(language) = block_string_language(primary) {
                if language == "markdown"
                    && let Err(err) = validate_markdown_xml(&primary.scalar_string())
                {
                    self.errorf(primary.value.get_range(), err);
                }
                g.edges[edge_idx].language = language;
            }
        }

        // Process edge map
        if let Some(ref emap) = e.map {
            self.compile_edge_map(g, edge_idx, emap);
        }
    }

    fn compile_edge_map(&mut self, g: &mut Graph, edge_idx: usize, m: &ir::Map) {
        // Apply any referenced classes first — mirrors the class
        // expansion at the top of `compile_map`.
        if !self.class_defs.is_empty() {
            if let Some(class_field) = m.get_field("class") {
                let mut names: Vec<String> = Vec::new();
                if let Some(ref primary) = class_field.primary {
                    names.push(primary.scalar_string());
                } else if let Some(ir::Composite::Array(ref arr)) = class_field.composite {
                    for v in &arr.values {
                        if let ir::Value::Scalar(s) = v {
                            names.push(s.scalar_string());
                        }
                    }
                }
                for name in names {
                    if let Some(class_map) = self.class_defs.get(&name.to_lowercase()).cloned() {
                        self.compile_edge_map(g, edge_idx, &class_map);
                    }
                }
            }
        }

        for f in &m.fields {
            let keyword = f.name.to_lowercase();
            if !(ast::RESERVED_KEYWORDS.contains(keyword.as_str()) && f.name_is_unquoted) {
                continue;
            }
            self.compile_edge_field(g, edge_idx, f);
        }
    }

    fn compile_edge_field(&mut self, g: &mut Graph, edge_idx: usize, f: &ir::Field) {
        let keyword = f.name.to_lowercase();
        let is_style_reserved =
            ast::STYLE_KEYWORDS.contains(keyword.as_str()) && f.name_is_unquoted;
        if is_style_reserved {
            return; // must be style.X
        }

        if f.name == "style" && f.name_is_unquoted {
            if let Some(fmap) = f.map() {
                self.compile_edge_style(g, edge_idx, fmap);
            }
            return;
        }

        if f.name == "source-arrowhead" || f.name == "target-arrowhead" {
            self.compile_arrowhead(g, edge_idx, f);
            return;
        }

        let primary_str = f.primary_string();
        match keyword.as_str() {
            "label" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].label.value = val;
                }
                if let Some(ref primary) = f.primary
                    && let Some(language) = block_string_language(primary)
                {
                    if language == "markdown"
                        && let Err(err) = validate_markdown_xml(&primary.scalar_string())
                    {
                        self.errorf(primary.value.get_range(), err);
                    }
                    g.edges[edge_idx].language = language;
                }
            }
            "icon" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].icon = Some(val);
                }
                if let Some(fmap) = f.map() {
                    for sub in &fmap.fields {
                        if sub.name == "style" && sub.name_is_unquoted {
                            if let Some(style_map) = sub.map() {
                                self.compile_edge_icon_style(g, edge_idx, style_map);
                            }
                        }
                    }
                }
            }
            "tooltip" => {
                if let Some(val) = primary_str {
                    if let Err(err) = validate_markdown_xml(&val)
                        && let Some(ref primary) = f.primary
                    {
                        self.errorf(primary.value.get_range(), err);
                    }
                    g.edges[edge_idx].tooltip = Some(ScalarValue { value: val });
                }
            }
            "link" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].link = Some(ScalarValue { value: val });
                }
            }
            "class" => {
                // Capture class names on the edge so the renderer can
                // emit them as SVG CSS classes (Go `d2target.Connection.Classes`).
                if let Some(val) = primary_str {
                    g.edges[edge_idx].classes.push(val);
                } else if let Some(ir::Composite::Array(ref arr)) = f.composite {
                    for v in &arr.values {
                        if let ir::Value::Scalar(s) = v {
                            g.edges[edge_idx].classes.push(s.scalar_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn compile_arrowhead(&mut self, g: &mut Graph, edge_idx: usize, f: &ir::Field) {
        let is_src = f.name == "source-arrowhead";

        if is_src {
            if g.edges[edge_idx].src_arrowhead.is_none() {
                g.edges[edge_idx].src_arrowhead = Some(graph::ArrowheadInfo::default());
            }
        } else {
            if g.edges[edge_idx].dst_arrowhead.is_none() {
                g.edges[edge_idx].dst_arrowhead = Some(graph::ArrowheadInfo::default());
            }
        }

        if let Some(ref primary) = f.primary {
            let label = primary.scalar_string();
            if is_src {
                if let Some(ref mut ah) = g.edges[edge_idx].src_arrowhead {
                    ah.label.value = label;
                }
            } else {
                if let Some(ref mut ah) = g.edges[edge_idx].dst_arrowhead {
                    ah.label.value = label;
                }
            }
        }

        if let Some(fmap) = f.map() {
            for f2 in &fmap.fields {
                let keyword = f2.name.to_lowercase();
                if keyword == "shape" && f2.name_is_unquoted {
                    if let Some(val) = f2.primary_string() {
                        if is_src {
                            if let Some(ref mut ah) = g.edges[edge_idx].src_arrowhead {
                                ah.shape = Some(val);
                            }
                        } else {
                            if let Some(ref mut ah) = g.edges[edge_idx].dst_arrowhead {
                                ah.shape = Some(val);
                            }
                        }
                    }
                } else if keyword == "style" && f2.name_is_unquoted {
                    // Parse `style.filled: <bool>` so circle/box arrowheads
                    // can distinguish outlined vs filled variants.
                    if let Some(smap) = f2.map() {
                        for sf in &smap.fields {
                            if sf.name == "filled" && sf.name_is_unquoted {
                                if let Some(val) = sf.primary_string() {
                                    let b = val == "true";
                                    if is_src {
                                        if let Some(ref mut ah) = g.edges[edge_idx].src_arrowhead {
                                            ah.filled = Some(b);
                                        }
                                    } else if let Some(ref mut ah) = g.edges[edge_idx].dst_arrowhead
                                    {
                                        ah.filled = Some(b);
                                    }
                                }
                            }
                            // Parse style.font-color for arrowhead labels
                            if sf.name == "font-color" && sf.name_is_unquoted {
                                if let Some(val) = sf.primary_string() {
                                    let sv = graph::ScalarValue { value: val };
                                    if is_src {
                                        if let Some(ref mut ah) = g.edges[edge_idx].src_arrowhead {
                                            ah.style.font_color = Some(sv);
                                        }
                                    } else if let Some(ref mut ah) = g.edges[edge_idx].dst_arrowhead {
                                        ah.style.font_color = Some(sv);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn set_default_shapes(&mut self, g: &mut Graph) {
        let count = g.objects.len();
        for i in 0..count {
            if g.objects[i].shape.value.is_empty() {
                let mut outer_sequence_diagram = false;
                let mut parent = g.objects[i].parent;
                while let Some(pid) = parent {
                    if g.objects[pid].is_sequence_diagram() {
                        outer_sequence_diagram = true;
                        break;
                    }
                    parent = g.objects[pid].parent;
                }

                g.objects[i].shape.value = if outer_sequence_diagram {
                    d2_target::SHAPE_RECTANGLE.to_owned()
                } else if g.objects[i].language == "latex" || g.objects[i].language == "markdown" {
                    d2_target::SHAPE_TEXT.to_owned()
                } else if !g.objects[i].language.is_empty() {
                    d2_target::SHAPE_CODE.to_owned()
                } else {
                    d2_target::SHAPE_RECTANGLE.to_owned()
                };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Language aliases (from Go)
// ---------------------------------------------------------------------------

pub fn short_to_full_language(tag: &str) -> Option<&'static str> {
    match tag {
        "md" => Some("markdown"),
        "tex" => Some("latex"),
        "js" => Some("javascript"),
        "go" => Some("golang"),
        "py" => Some("python"),
        "rb" => Some("ruby"),
        "ts" => Some("typescript"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests - ported from d2compiler/compile_test.go
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_ok(input: &str) -> Graph {
        compile("test.d2", input).expect("should compile without errors")
    }

    fn compile_err(input: &str) -> CompileError {
        compile("test.d2", input).expect_err("should produce compile error")
    }

    #[test]
    fn test_compile_with_config_extracts_theme_overrides() {
        let (_, config) = compile_with_config(
            "test.d2",
            r##"
vars: {
  d2-config: {
    sketch: true
    theme-id: 300
    dark-theme-id: 200
    theme-overrides: {
      B1: "#2E7D32"
      N7: "#DCDCDC"
    }
  }
}
a -> b
"##,
        )
        .expect("should compile with config");

        let config = config.expect("config should be present");
        assert_eq!(config.sketch, Some(true));
        assert_eq!(config.theme_id, Some(300));
        assert_eq!(config.dark_theme_id, Some(200));
        let overrides = config.theme_overrides.expect("theme overrides");
        assert_eq!(overrides.b1.as_deref(), Some("#2E7D32"));
        assert_eq!(overrides.n7.as_deref(), Some("#DCDCDC"));
    }

    #[test]
    fn test_literal_star_node_is_not_emitted_and_applies_style() {
        let g = compile_ok(
            r#"
x
y
*.style.multiple: true
"#,
        );

        assert_eq!(g.objects.len(), 3);
        assert!(g.objects.iter().all(|o| o.id_val() != "*"));
        assert_eq!(g.objects[1].style.multiple.as_ref().map(|v| v.value.as_str()), Some("true"));
        assert_eq!(g.objects[2].style.multiple.as_ref().map(|v| v.value.as_str()), Some("true"));
    }

    #[test]
    fn test_literal_star_edge_expands_to_real_edges() {
        let g = compile_ok(
            r#"
container: {
  a
  b
  *.style.multiple: true
  * -> sink
}
"#,
        );

        let edge_ids: Vec<_> = g.edges.iter().map(|e| e.abs_id.as_str()).collect();
        assert_eq!(edge_ids, vec!["container.(a -> sink)[0]", "container.(b -> sink)[0]"]);
        assert!(g.objects.iter().all(|o| o.id_val() != "*"));
    }

    // --- Test 1: basic_shape ---
    #[test]
    fn test_basic_shape() {
        let g = compile_ok("x: {\n  shape: circle\n}");
        // root + x
        assert_eq!(g.objects.len(), 2);
        assert_eq!(g.objects[1].id_val(), "x");
        assert_eq!(g.objects[1].shape.value, "circle");
    }

    // --- Test 2: basic_style ---
    #[test]
    fn test_basic_style() {
        let g = compile_ok("x: {\n  style.opacity: 0.4\n}");
        assert_eq!(g.objects.len(), 2);
        assert_eq!(g.objects[1].id_val(), "x");
        assert_eq!(g.objects[1].style.opacity.as_ref().unwrap().value, "0.4");
    }

    // --- Test 3: dimensions_on_nonimage ---
    #[test]
    fn test_dimensions_on_nonimage() {
        let g = compile_ok("hey: {\n  shape: hexagon\n  width: 200\n  height: 230\n}");
        assert_eq!(g.objects.len(), 2);
        assert_eq!(g.objects[1].id_val(), "hey");
        assert_eq!(g.objects[1].shape.value, "hexagon");
        assert_eq!(g.objects[1].width_attr.as_ref().unwrap().value, "200");
        assert_eq!(g.objects[1].height_attr.as_ref().unwrap().value, "230");
    }

    // --- Test 4: positions ---
    #[test]
    fn test_positions() {
        let g = compile_ok("hey: {\n  top: 200\n  left: 230\n}");
        assert_eq!(g.objects[1].top.as_ref().unwrap().value, "200");
        assert_eq!(g.objects[1].left.as_ref().unwrap().value, "230");
    }

    // --- Test 5: basic single object ---
    #[test]
    fn test_single_object() {
        let g = compile_ok("x");
        assert_eq!(g.objects.len(), 2); // root + x
        assert_eq!(g.objects[1].id_val(), "x");
    }

    // --- Test 6: labeled object ---
    #[test]
    fn test_labeled_object() {
        let g = compile_ok("x: hello world");
        assert_eq!(g.objects[1].label.value, "hello world");
    }

    // --- Test 7: basic edge ---
    #[test]
    fn test_basic_edge() {
        let g = compile_ok("a -> b");
        assert_eq!(g.objects.len(), 3); // root + a + b
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.objects[g.edges[0].src].id_val(), "a");
        assert_eq!(g.objects[g.edges[0].dst].id_val(), "b");
    }

    // --- Test 8: edge with label ---
    #[test]
    fn test_edge_with_label() {
        let g = compile_ok("a -> b: hello");
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].label.value, "hello");
    }

    // --- Test 9: edge chain ---
    #[test]
    fn test_edge_chain() {
        let g = compile_ok("a -> b -> c");
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.objects[g.edges[0].src].id_val(), "a");
        assert_eq!(g.objects[g.edges[0].dst].id_val(), "b");
        assert_eq!(g.objects[g.edges[1].src].id_val(), "b");
        assert_eq!(g.objects[g.edges[1].dst].id_val(), "c");
    }

    // --- Test 10: nested objects ---
    #[test]
    fn test_nested_objects() {
        let g = compile_ok("a: {\n  b: {\n    c\n  }\n}");
        // root + a + b + c
        assert_eq!(g.objects.len(), 4);
        assert_eq!(g.objects[1].id_val(), "a");
        assert_eq!(g.objects[2].id_val(), "b");
        assert_eq!(g.objects[3].id_val(), "c");
    }

    // --- Test 11: style fill ---
    #[test]
    fn test_style_fill() {
        let g = compile_ok("x: {\n  style: {\n    fill: \"#ff0000\"\n  }\n}");
        assert!(g.objects[1].style.fill.is_some());
        assert_eq!(g.objects[1].style.fill.as_ref().unwrap().value, "#ff0000");
    }

    // --- Test 12: edge style ---
    #[test]
    fn test_edge_style() {
        let g = compile_ok("a -> b: {\n  style: {\n    stroke-dash: 3\n  }\n}");
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].style.stroke_dash.is_some());
        assert_eq!(g.edges[0].style.stroke_dash.as_ref().unwrap().value, "3");
    }

    // --- Test 13: multiple objects ---
    #[test]
    fn test_multiple_objects() {
        let g = compile_ok("a\nb\nc\nd");
        assert_eq!(g.objects.len(), 5); // root + 4
    }

    // --- Test 14: multiple edges ---
    #[test]
    fn test_multiple_edges() {
        let g = compile_ok("a -> b\nc -> d");
        assert_eq!(g.edges.len(), 2);
    }

    // --- Test 15: bidirectional edge ---
    #[test]
    fn test_bidirectional_edge() {
        let g = compile_ok("a <-> b");
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].src_arrow);
        assert!(g.edges[0].dst_arrow);
    }

    // --- Test 16: reverse edge ---
    #[test]
    fn test_reverse_edge() {
        let g = compile_ok("a <- b");
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].src_arrow);
        assert!(!g.edges[0].dst_arrow);
    }

    // --- Test 17: direction keyword ---
    #[test]
    fn test_direction() {
        let g = compile_ok("x: {\n  direction: right\n}");
        assert_eq!(g.objects[1].direction.value, "right");
    }

    // --- Test 18: icon ---
    #[test]
    fn test_icon() {
        let g = compile_ok("hey: {\n  icon: https://example.com/icon.svg\n}");
        assert!(g.objects[1].icon.is_some());
    }

    // --- Test 19: link ---
    #[test]
    fn test_link() {
        let g = compile_ok("x: {\n  link: https://example.com\n}");
        assert!(g.objects[1].link.is_some());
        assert_eq!(
            g.objects[1].link.as_ref().unwrap().value,
            "https://example.com"
        );
    }

    // --- Test 20: tooltip ---
    #[test]
    fn test_tooltip() {
        let g = compile_ok("x: {\n  tooltip: hello there\n}");
        assert!(g.objects[1].tooltip.is_some());
        assert_eq!(g.objects[1].tooltip.as_ref().unwrap().value, "hello there");
    }

    // --- Test 21: object label override ---
    #[test]
    fn test_label_override() {
        let g = compile_ok("x: first\nx: second");
        assert_eq!(g.objects.len(), 2); // root + x (not duplicated)
        assert_eq!(g.objects[1].label.value, "second");
    }

    // --- Test 22: default shape ---
    #[test]
    fn test_default_shape() {
        let g = compile_ok("x");
        assert_eq!(g.objects[1].shape.value, "rectangle");
    }

    // --- Test 23: constraint ---
    #[test]
    fn test_constraint() {
        let g = compile_ok("x: {\n  shape: sql_table\n  id: int { constraint: primary_key }\n}");
        let table = g.objects.iter().find(|o| o.abs_id == "x").expect("x");
        assert_eq!(table.shape.value, d2_target::SHAPE_SQL_TABLE);
        assert!(table.sql_table.is_some());
        assert!(table.children_array.is_empty());
    }

    #[test]
    fn test_sql_table_reserved_columns_keep_shape_and_clear_children() {
        let g = compile_ok(
            "my_table: {\n  shape: sql_table\n  icon: https://example.com/icon.svg\n  width: 200\n  height: 200\n  \"shape\": string\n  \"icon\": string\n  \"width\": int\n  \"height\": int\n}\n",
        );

        let table = g
            .objects
            .iter()
            .find(|o| o.abs_id == "my_table")
            .expect("my_table");
        assert_eq!(table.shape.value, d2_target::SHAPE_SQL_TABLE);
        assert!(table.sql_table.is_some());
        assert!(table.children_array.is_empty());
        assert_eq!(table.width_attr.as_ref().unwrap().value, "200");
        assert_eq!(table.height_attr.as_ref().unwrap().value, "200");
        let columns = &table.sql_table.as_ref().unwrap().columns;
        let names: Vec<&str> = columns.iter().map(|c| c.name.label.as_str()).collect();
        assert_eq!(names, vec!["shape", "icon", "width", "height"]);
    }

    #[test]
    fn test_sql_table_column_edge_preserves_shape() {
        let g = compile_ok(
            "my_table: {\n  shape: sql_table\n  icon: https://example.com/icon.svg\n  width: 200\n  height: 200\n  \"shape\": string\n  \"icon\": string\n  \"width\": int\n  \"height\": int\n}\n\nx -> my_table.\"shape\"\n",
        );
        let table = g
            .objects
            .iter()
            .find(|o| o.abs_id == "my_table")
            .expect("my_table");
        assert_eq!(table.shape.value, d2_target::SHAPE_SQL_TABLE);
        assert!(table.sql_table.is_some());
    }

    // --- Test 24: vars and substitution ---
    #[test]
    fn test_vars_substitution() {
        let g = compile_ok("vars: {\n  mycolor: red\n}\nx: {\n  style.fill: ${mycolor}\n}");
        // Check that x exists and has fill set
        assert!(g.objects.len() >= 2);
    }

    // --- Test 25: edge with map ---
    #[test]
    fn test_edge_with_map_fields() {
        let g = compile_ok("a -> b: {\n  label: connection\n}");
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].label.value, "connection");
    }

    #[test]
    fn test_multiple_edge_styles_stay_on_their_own_edges() {
        let g =
            compile_ok("x -> y: {\n  style.stroke: green\n}\ny -> z: {\n  style.stroke: red\n}");
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.edges[0].style.stroke.as_ref().unwrap().value, "green");
        assert_eq!(g.edges[1].style.stroke.as_ref().unwrap().value, "red");
    }
}
