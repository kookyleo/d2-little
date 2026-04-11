//! d2-lib: top-level entry point that ties the entire d2-little pipeline together.
//!
//! Pipeline: D2 source text -> AST -> IR -> Graph -> (theme + dimensions + layout) -> Diagram -> SVG.
//!
//! Ported from Go `d2lib/d2.go`.

use d2_fonts::{self, FONT_SIZE_M, FontFamily, FontStyle};
use d2_graph::{self, Graph};
use d2_svg_render::{self, RenderOpts};
use d2_target;
use d2_textmeasure;
use d2_themes;

// ---------------------------------------------------------------------------
// Constants (matching Go d2graph constants)
// ---------------------------------------------------------------------------

const DEFAULT_SHAPE_SIZE: f64 = 100.0;
const MIN_SHAPE_SIZE: f64 = 5.0;
/// Padding added around label text inside a shape.
const INNER_LABEL_PADDING: f64 = 16.0;

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
    let mut g = d2_compiler::compile("", input).map_err(|e| format!("{}", e))?;

    // Step 2: apply theme
    let theme_id = opts.theme_id.unwrap_or(0);
    if let Some(theme) = d2_themes::catalog::find(theme_id) {
        g.theme = Some(theme.clone());
    }

    // Step 3: set dimensions on objects using text measurement
    let mut ruler = match opts.ruler {
        Some(ref _r) => None, // we'll use the provided ruler below
        None => Some(d2_textmeasure::Ruler::new().map_err(|e| format!("ruler init: {}", e))?),
    };
    let ruler_ref: &mut d2_textmeasure::Ruler = if let Some(ref mut r) = ruler {
        r
    } else {
        // This branch is unreachable given the logic above, but let's be safe.
        // We always create a ruler if none is provided.
        return Err("no ruler available".to_string());
    };

    set_dimensions(&mut g, ruler_ref)?;

    // Step 4: layout
    d2_dagre_layout::layout(&mut g, None)?;

    // Step 5: export
    let diagram = d2_exporter::export(&g, None, None)?;

    // Step 6: render
    let render_opts = RenderOpts {
        theme_id: Some(theme_id),
        dark_theme_id: opts.dark_theme_id,
        pad: opts.pad,
        sketch: if opts.sketch { Some(true) } else { None },
        center: if opts.center { Some(true) } else { None },
        ..Default::default()
    };
    let svg = d2_svg_render::render(&diagram, &render_opts)?;

    Ok((diagram, svg))
}

/// Convenience function: D2 source text -> SVG bytes with default options.
pub fn d2_to_svg(input: &str) -> Result<Vec<u8>, String> {
    let opts = CompileOptions::default();
    let (_, svg) = compile(input, &opts)?;
    Ok(svg)
}

// ---------------------------------------------------------------------------
// set_dimensions: measure text and assign object/edge dimensions
// ---------------------------------------------------------------------------

/// Measure label text for each object and edge, then set their width/height.
///
/// This is a simplified port of Go's `Graph.SetDimensions`.
fn set_dimensions(g: &mut Graph, ruler: &mut d2_textmeasure::Ruler) -> Result<(), String> {
    let font_family = if g.theme.as_ref().is_some_and(|t| t.special_rules.mono) {
        FontFamily::SourceCodePro
    } else {
        FontFamily::SourceSansPro
    };

    // Process objects (skip root at index 0)
    let count = g.objects.len();
    for i in 1..count {
        let label = g.objects[i].label.value.clone();
        let shape = g.objects[i].shape.value.clone();

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

        // Determine font style
        let is_bold = g.objects[i]
            .style
            .bold
            .as_ref()
            .is_some_and(|v| v.value == "true");
        let is_italic = g.objects[i]
            .style
            .italic
            .as_ref()
            .is_some_and(|v| v.value == "true");
        let font_size: i32 = g.objects[i]
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

        let font = d2_fonts::Font::new(font_family, font_style, font_size);

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
        let (tw, th) = ruler.measure(font, &label);
        g.objects[i].label_dimensions = d2_graph::Dimensions {
            width: tw,
            height: th,
        };

        // Compute shape dimensions from label + padding
        let pad_w = INNER_LABEL_PADDING * 2.0;
        let pad_h = INNER_LABEL_PADDING * 2.0;
        let mut w = (tw as f64 + pad_w).max(MIN_SHAPE_SIZE);
        let mut h = (th as f64 + pad_h).max(MIN_SHAPE_SIZE);

        // Apply desired dimensions
        if desired_width > 0 {
            w = w.max(desired_width as f64);
        }
        if desired_height > 0 {
            h = h.max(desired_height as f64);
        }

        // Square and circle shapes must be equal width/height
        if shape == "circle" || shape == "square" {
            let side = w.max(h);
            w = side;
            h = side;
        }

        g.objects[i].width = w;
        g.objects[i].height = h;
        g.objects[i].update_box();
    }

    // Process edges: measure edge labels
    let edge_count = g.edges.len();
    for i in 0..edge_count {
        let label = g.edges[i].label.value.clone();
        if label.is_empty() {
            continue;
        }

        let is_bold = g.edges[i]
            .style
            .bold
            .as_ref()
            .is_some_and(|v| v.value == "true");
        let is_italic = g.edges[i]
            .style
            .italic
            .as_ref()
            .is_some_and(|v| v.value == "true");
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

        let font = d2_fonts::Font::new(font_family, font_style, font_size);
        let (tw, th) = ruler.measure(font, &label);
        g.edges[i].label_dimensions = d2_graph::Dimensions {
            width: tw,
            height: th,
        };
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

#[cfg(test)]
mod overflow_tests {
    #[test]
    fn binary_tree_pipeline() {
        let script = "a -> b\na -> c\nb -> d\nb -> e\nc -> f\nc -> g\nd -> h\nd -> i\ne -> j\ne -> k\nf -> l\nf -> m\ng -> n\ng -> o\n";
        eprintln!("[1] Compiling...");
        let mut g = d2_compiler::compile("", script).unwrap();
        eprintln!("[1] OK: {} objects, {} edges", g.objects.len(), g.edges.len());

        eprintln!("[2] Theme...");
        let theme = d2_themes::catalog::find(0).cloned();
        g.theme = theme;
        eprintln!("[2] OK");

        eprintln!("[3] set_dimensions...");
        let mut ruler = d2_textmeasure::Ruler::new().unwrap();
        super::set_dimensions(&mut g, &mut ruler).unwrap();
        eprintln!("[3] OK");

        eprintln!("[4] dagre layout...");
        d2_dagre_layout::layout(&mut g, None).unwrap();
        eprintln!("[4] OK");

        eprintln!("[5] export...");
        let diagram = d2_exporter::export(&g, None, None).unwrap();
        eprintln!("[5] OK");

        eprintln!("[6] svg render...");
        let opts = d2_svg_render::RenderOpts::default();
        let svg = d2_svg_render::render(&diagram, &opts).unwrap();
        eprintln!("[6] OK: {} bytes", svg.len());
    }
}
