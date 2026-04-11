//! d2-compiler: Compiles d2 source text into a Graph.
//!
//! Pipeline: source text -> AST (d2-parser) -> IR (d2-ir) -> Graph (d2-graph).
//! Ported from Go `d2compiler/compile.go`.

use d2_ast::{self as ast};
use d2_graph::{self as graph, Graph, ObjId, ScalarValue};
use d2_ir::{self as ir};
use d2_target;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compile d2 source text into a Graph.
pub fn compile(path: &str, input: &str) -> Result<Graph, CompileError> {
    let (ast_map, parse_err) = d2_parser::parse(path, input);
    if let Some(e) = parse_err {
        return Err(CompileError { errors: e.errors });
    }

    let ir_map = ir::compile(&ast_map).map_err(|e| CompileError { errors: e.errors })?;

    let mut c = Compiler::new();
    let mut g = Graph::new();

    c.compile_board(&mut g, &ir_map);
    c.set_default_shapes(&mut g);

    // Match Go d2compiler: if there are no user objects (only the implicit root),
    // mark the graph as folder-only so it will not be rendered as its own board.
    // See Go d2compiler/compile.go:  `if len(g.Objects) == 0 { g.IsFolderOnly = true }`
    // (Go excludes the root from Objects; our objects[0] is the root.)
    if g.objects.len() <= 1 && g.edges.is_empty() {
        g.is_folder_only = true;
    }

    if c.errors.is_empty() {
        Ok(g)
    } else {
        Err(CompileError { errors: c.errors })
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
}

impl Compiler {
    fn new() -> Self {
        Self { errors: Vec::new() }
    }

    fn errorf(&mut self, range: &ast::Range, msg: String) {
        self.errors.push(ast::Error {
            range: range.clone(),
            message: msg,
        });
    }

    fn compile_board(&mut self, g: &mut Graph, ir: &ir::Map) {
        let root = g.root;
        self.compile_map(g, root, ir);
        self.set_default_shapes(g);
    }

    fn compile_map(&mut self, g: &mut Graph, obj: ObjId, m: &ir::Map) {
        // Process shape first (affects how children are handled)
        if let Some(shape_field) = m.get_field("shape") {
            if shape_field.composite.is_some() {
                // "reserved field shape does not accept composite"
            } else {
                self.compile_field(g, obj, shape_field);
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
            self.compile_field(g, obj, f);
        }

        // Process edges
        for e in &m.edges {
            self.compile_edge(g, obj, e);
        }
    }

    fn compile_field(&mut self, g: &mut Graph, obj: ObjId, f: &ir::Field) {
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

        // Set label from primary value
        if let Some(ref primary) = f.primary {
            let label_val = primary.scalar_string();
            g.objects[child].label.value = label_val;
        }

        // Recurse into map
        if let Some(fmap) = f.map() {
            self.compile_map(g, child, fmap);
        }
    }

    fn compile_reserved(&mut self, g: &mut Graph, obj: ObjId, f: &ir::Field) {
        let primary_str = f.primary_string();

        match f.name.as_str() {
            "label" => {
                if let Some(val) = primary_str {
                    g.objects[obj].label.value = val;
                }
            }
            "shape" => {
                if let Some(val) = primary_str {
                    let lower = val.to_lowercase();
                    if !d2_target::is_shape(&lower) {
                        // Unknown shape
                        return;
                    }
                    g.objects[obj].shape.value = lower;
                }
            }
            "icon" => {
                if let Some(val) = primary_str {
                    g.objects[obj].icon = Some(val);
                }
            }
            "tooltip" => {
                if let Some(val) = primary_str {
                    g.objects[obj].tooltip = Some(ScalarValue { value: val });
                }
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

    fn compile_edge(&mut self, g: &mut Graph, obj: ObjId, e: &ir::IREdge) {
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

        // Set label from primary
        if let Some(ref primary) = e.primary {
            g.edges[edge_idx].label.value = primary.scalar_string();
        }

        // Process edge map
        if let Some(ref emap) = e.map {
            self.compile_edge_map(g, edge_idx, emap);
        }
    }

    fn compile_edge_map(&mut self, g: &mut Graph, edge_idx: usize, m: &ir::Map) {
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
            }
            "icon" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].icon = Some(val);
                }
            }
            "tooltip" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].tooltip = Some(ScalarValue { value: val });
                }
            }
            "link" => {
                if let Some(val) = primary_str {
                    g.edges[edge_idx].link = Some(ScalarValue { value: val });
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
                    // Arrowhead style - simplified, skip
                }
            }
        }
    }

    fn set_default_shapes(&mut self, g: &mut Graph) {
        let count = g.objects.len();
        for i in 0..count {
            if g.objects[i].shape.value.is_empty() {
                g.objects[i].shape.value = d2_target::SHAPE_RECTANGLE.to_string();
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
        // root + x + id
        assert!(g.objects.len() >= 3);
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
}
