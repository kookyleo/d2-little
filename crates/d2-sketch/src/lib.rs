//! d2-sketch: sketch/hand-drawn rendering via rough.js running in rquickjs.
//!
//! Ported verbatim from Go `d2renderers/d2sketch/sketch.go`.  We embed the
//! same `rough.js` + `setup.js` used by the Go implementation so the primitive
//! paths come out byte-identical (after truncating trailing float decimals to
//! 6 places, which the Go code does to paper over Math.sin/cos drift between
//! JS engines).
//!
//! License: Apache-2.0.

use std::cell::RefCell;
use std::fmt::Write as _;

use rquickjs::{Context, Runtime};
use serde::Deserialize;

use d2_geo::{Point, Segment};
use d2_target::{Arrowhead, Connection, Shape, INNER_BORDER_OFFSET};
use d2_themes::ThemableElement;

// ---------------------------------------------------------------------------
// Embedded JS / patterns
// ---------------------------------------------------------------------------

const ROUGH_JS: &str = include_str!("../assets/rough.js");
const SETUP_JS: &str = include_str!("../assets/setup.js");
const STREAKS_TEMPLATE: &str = include_str!("../assets/streaks.txt");

/// rough.js options appended verbatim to every primitive call.  Matches the
/// `baseRoughProps` constant in Go `sketch.go` byte-for-byte (note the
/// trailing comma on each line — the Go string literal keeps it).
const BASE_ROUGH_PROPS: &str = "fillWeight: 2.0,
hachureGap: 16,
fillStyle: \"solid\",
bowing: 2,
seed: 1,";

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Holds a rquickjs runtime with rough.js + setup.js evaluated once.  Reuse
/// across every shape/connection in a single diagram render so we avoid
/// paying the ~tens-of-ms JIT cost per primitive.
///
/// Not `Send`/`Sync` because rquickjs `Runtime` is single-threaded.
pub struct SketchRunner {
    // Kept alive for the Context's lifetime; never read directly.
    _runtime: Runtime,
    // Interior mutability so shape/connection ports can call `.eval`
    // through a shared `&SketchRunner` without cascading `&mut` all the way up
    // through the renderer.
    context: RefCell<Context>,
}

impl SketchRunner {
    /// Create a new runtime and evaluate `rough.js` + `setup.js`.  Errors if
    /// either script fails (unlikely — both are vendored and deterministic).
    pub fn new() -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| format!("create rquickjs runtime: {e:?}"))?;
        let context = Context::full(&runtime)
            .map_err(|e| format!("create rquickjs context: {e:?}"))?;

        let result: Result<(), String> = context.with(|ctx| {
            let _: () = ctx
                .eval(ROUGH_JS)
                .map_err(|e| format!("eval rough.js: {e:?}"))?;
            let _: () = ctx
                .eval(SETUP_JS)
                .map_err(|e| format!("eval setup.js: {e:?}"))?;
            Ok(())
        });
        result?;

        Ok(Self {
            _runtime: runtime,
            context: RefCell::new(context),
        })
    }

    /// Evaluate `js` in the persistent context, expecting unit return value.
    fn eval_unit(&self, js: &str) -> Result<(), String> {
        let ctx = self.context.borrow();
        ctx.with(|ctx| {
            let _: () = ctx
                .eval::<(), _>(js)
                .map_err(|e| format!("eval failed: {e:?}"))?;
            Ok(())
        })
    }

    /// Evaluate `js` and return its result as a String (used for
    /// `JSON.stringify(node.children)` extraction).
    fn eval_string(&self, js: &str) -> Result<String, String> {
        let ctx = self.context.borrow();
        ctx.with(|ctx| {
            ctx.eval::<String, _>(js)
                .map_err(|e| format!("eval string failed: {e:?}"))
        })
    }
}

// ---------------------------------------------------------------------------
// rough.js result parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
struct RoughAttrs {
    d: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
struct RoughStyle {
    #[serde(default)]
    stroke: String,
    #[serde(default, rename = "strokeWidth")]
    stroke_width: String,
    #[serde(default)]
    fill: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
struct RoughPath {
    attrs: RoughAttrs,
    style: RoughStyle,
}

impl RoughPath {
    /// CSS `style` fragment that mirrors Go `roughPath.StyleCSS`:
    /// only `stroke-width` is emitted (when non-empty).
    fn style_css(&self) -> String {
        if self.style.stroke_width.is_empty() {
            String::new()
        } else {
            format!("stroke-width:{};", self.style.stroke_width)
        }
    }
}

/// Eval `js` and pull `node.children` back as a JSON array of RoughPath,
/// then truncate every `(\d+).(\d+)` literal in each `attrs.d` to at most
/// 6 decimal places.  Mirrors Go `extractRoughPaths`.
fn extract_rough_paths(runner: &SketchRunner, js: &str) -> Result<Vec<RoughPath>, String> {
    runner.eval_unit(js)?;
    let json = runner.eval_string("JSON.stringify(node.children, null, '  ')")?;

    let mut paths: Vec<RoughPath> = serde_json::from_str(&json)
        .map_err(|e| format!("parse rough paths JSON: {e}"))?;

    for p in &mut paths {
        p.attrs.d = truncate_floats(&p.attrs.d);
    }
    Ok(paths)
}

/// Extract only the path `d` attributes from a rough.js result, after float
/// truncation.  Mirrors Go `computeRoughPathData`.
fn compute_rough_path_data(runner: &SketchRunner, js: &str) -> Result<Vec<String>, String> {
    let paths = extract_rough_paths(runner, js)?;
    Ok(paths.into_iter().map(|p| p.attrs.d).collect())
}

/// Same as `extract_rough_paths`, alias matching Go name.
fn compute_rough_paths(runner: &SketchRunner, js: &str) -> Result<Vec<RoughPath>, String> {
    extract_rough_paths(runner, js)
}

/// Truncate every decimal literal to at most 6 decimal places.  Matches
/// Go's `floatRE = regexp.MustCompile(\`(\d+)\.(\d+)\`)` + ReplaceAllStringFunc:
/// the sequence is truncation (not rounding) to 6 digits past the dot.
///
/// Implemented as a small state machine rather than a full regex — the input
/// is a single SVG path string, so this hot path gets called once per shape.
fn truncate_floats(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Find a run of digits followed by `.` followed by more digits; that
        // is Go's `(\d+)\.(\d+)` match.
        if c.is_ascii_digit() {
            let int_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let dot_pos = i;
                i += 1; // skip '.'
                let dec_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let decimal_len = i - dec_start;
                let keep = decimal_len.min(6);
                // Push integer part + '.' + up to 6 decimals; drop the rest.
                out.push_str(&input[int_start..=dot_pos]);
                out.push_str(&input[dec_start..dec_start + keep]);
            } else {
                out.push_str(&input[int_start..i]);
            }
        } else {
            // Push one byte (ASCII-safe slice).
            out.push(c as char);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Fill patterns (streaks)
// ---------------------------------------------------------------------------

/// Append `<defs>…</defs>` with the four streak fill patterns used by sketched
/// overlays, keyed by `diagramHash`.  Only emits each pattern if the rendered
/// SVG source already references it (matches Go DefineFillPatterns).
pub fn define_fill_patterns(buf: &mut String, diagram_hash: &str) {
    let source_snapshot = buf.clone();
    buf.push_str("<defs>");
    for (lc, fill) in &[
        ("bright", "rgba(0, 0, 0, 0.1)"),
        ("normal", "rgba(0, 0, 0, 0.16)"),
        ("dark", "rgba(0, 0, 0, 0.32)"),
        ("darker", "rgba(255, 255, 255, 0.24)"),
    ] {
        let trigger = format!("url(#streaks-{lc}-{diagram_hash})");
        if source_snapshot.contains(&trigger) {
            // Go uses fmt.Fprintf(buf, streaks, lc, diagramHash, fill)
            // which replaces the first three %s in streaks.txt in order.
            buf.push_str(&format_streaks(STREAKS_TEMPLATE, lc, diagram_hash, fill));
        }
    }
    buf.push_str("</defs>");
}

fn format_streaks(template: &str, a: &str, b: &str, c: &str) -> String {
    // streaks.txt uses three `%s` placeholders in order.
    let mut out = String::with_capacity(template.len() + 64);
    let mut idx = 0;
    let bytes = template.as_bytes();
    let mut i = 0;
    let substs = [a, b, c];
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() && bytes[i + 1] == b's' {
            if idx < substs.len() {
                out.push_str(substs[idx]);
                idx += 1;
            }
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shape/connection helpers
// ---------------------------------------------------------------------------

fn shape_theme_fill_stroke(shape: &Shape) -> (String, String) {
    if shape.type_ == d2_target::SHAPE_CLASS || shape.type_ == d2_target::SHAPE_SQL_TABLE {
        (shape.stroke.clone(), shape.fill.clone())
    } else {
        (shape.fill.clone(), shape.stroke.clone())
    }
}

fn shape_css_style(s: &Shape) -> String {
    let mut out = format!("stroke-width:{};", s.stroke_width);
    if s.stroke_dash != 0.0 {
        let (dash, gap) =
            d2_svg_path::get_stroke_dash_attributes(s.stroke_width as f64, s.stroke_dash);
        write!(out, "stroke-dasharray:{:.6},{:.6};", dash, gap).unwrap();
    }
    out
}

fn connection_css_style(c: &Connection) -> String {
    let mut out = format!("stroke-width:{};", c.stroke_width);
    let mut stroke_dash = c.stroke_dash;
    if stroke_dash == 0.0 && c.animated {
        stroke_dash = 5.0;
    }
    if stroke_dash != 0.0 {
        let (dash, gap) =
            d2_svg_path::get_stroke_dash_attributes(c.stroke_width as f64, stroke_dash);
        write!(out, "stroke-dasharray:{:.6},{:.6};", dash, gap).unwrap();
        if c.animated {
            let mut dash_offset: f64 = -10.0;
            if c.src_arrow != Arrowhead::None && c.dst_arrow == Arrowhead::None {
                dash_offset = 10.0;
            }
            write!(out, "stroke-dashoffset:{:.6};", dash_offset * (dash + gap)).unwrap();
            write!(
                out,
                "animation: dashdraw {:.6}s linear infinite;",
                gap * 0.5
            )
            .unwrap();
        }
    }
    out
}

/// Append the `sketch-overlay-<category>` class to `el` based on `fill`'s
/// luminance and render.  Mirrors Go `ThemableSketchOverlay.Render`.
///
/// Go unconditionally concatenates with a leading space (`" sketch-overlay-X"`)
/// so the class attribute always has a leading space even when `class_name`
/// was previously empty — mirror that byte-for-byte.
fn render_sketch_overlay(el: &mut ThemableElement, fill: &str) -> Result<String, String> {
    if d2_color::is_theme_color(fill) {
        el.class_name.push_str(&format!(" sketch-overlay-{fill}"));
    } else {
        let lc = d2_color::luminance_category(fill)
            .map_err(|e| format!("luminance_category({fill}): {e}"))?;
        el.class_name.push_str(&format!(" sketch-overlay-{}", lc.as_str()));
    }
    Ok(el.render())
}

// ---------------------------------------------------------------------------
// Shape primitives
// ---------------------------------------------------------------------------

/// `d2sketch.Rect`: rough-rendered rectangle with streak overlay.
pub fn rect(runner: &SketchRunner, shape: &Shape, diagram_hash: &str) -> Result<String, String> {
    let js = format!(
        "node = rc.rectangle(0, 0, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width, shape.height, shape.stroke_width, BASE_ROUGH_PROPS
    );
    let paths = compute_rough_path_data(runner, &js)?;
    let (fill, stroke) = shape_theme_fill_stroke(shape);

    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    path_el.fill = fill.clone();
    path_el.stroke = stroke.clone();
    path_el.fill_pattern = shape.fill_pattern.clone();
    path_el.class_name = "shape".to_owned();
    path_el.style = shape_css_style(shape);

    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            path_el.mask = format!("url(#{diagram_hash})");
        }
    }

    let mut out = String::new();
    for p in paths {
        path_el.d = p;
        out.push_str(&path_el.render());
    }

    let mut so_el = ThemableElement::new("rect", None);
    so_el.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    so_el.width = Some(shape.width as f64);
    so_el.height = Some(shape.height as f64);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            so_el.mask = format!("url(#{diagram_hash})");
        }
    }
    out.push_str(&render_sketch_overlay(&mut so_el, &fill)?);
    Ok(out)
}

/// `d2sketch.DoubleRect`: two concentric rough rectangles + streak overlay.
pub fn double_rect(
    runner: &SketchRunner,
    shape: &Shape,
    diagram_hash: &str,
) -> Result<String, String> {
    let big = format!(
        "node = rc.rectangle(0, 0, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width, shape.height, shape.stroke_width, BASE_ROUGH_PROPS
    );
    let paths_big = compute_rough_path_data(runner, &big)?;
    let small = format!(
        "node = rc.rectangle(0, 0, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width - INNER_BORDER_OFFSET * 2,
        shape.height - INNER_BORDER_OFFSET * 2,
        shape.stroke_width,
        BASE_ROUGH_PROPS
    );
    let paths_small = compute_rough_path_data(runner, &small)?;

    let (fill, stroke) = shape_theme_fill_stroke(shape);
    let mut out = String::new();

    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    path_el.fill = fill.clone();
    path_el.stroke = stroke.clone();
    path_el.fill_pattern = shape.fill_pattern.clone();
    path_el.class_name = "shape".to_owned();
    path_el.style = shape_css_style(shape);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            path_el.mask = format!("url(#{diagram_hash})");
        }
    }
    for p in paths_big {
        path_el.d = p;
        out.push_str(&path_el.render());
    }

    let mut inner = ThemableElement::new("path", None);
    inner.set_translate(
        (shape.pos.x + INNER_BORDER_OFFSET) as f64,
        (shape.pos.y + INNER_BORDER_OFFSET) as f64,
    );
    let (_, stroke_i) = shape_theme_fill_stroke(shape);
    inner.fill = "transparent".to_owned();
    inner.stroke = stroke_i;
    inner.class_name = "shape".to_owned();
    inner.style = shape_css_style(shape);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            inner.mask = format!("url(#{diagram_hash})");
        }
    }
    for p in paths_small {
        inner.d = p;
        out.push_str(&inner.render());
    }

    let mut so = ThemableElement::new("rect", None);
    so.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    so.width = Some(shape.width as f64);
    so.height = Some(shape.height as f64);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            so.mask = format!("url(#{diagram_hash})");
        }
    }
    // NOTE: Go passes `shape.Fill` (not the swapped pair) for DoubleRect's
    // overlay.  Mirror exactly — see sketch.go line 188.
    out.push_str(&render_sketch_overlay(&mut so, &shape.fill)?);
    Ok(out)
}

/// `d2sketch.Oval`.
pub fn oval(runner: &SketchRunner, shape: &Shape, diagram_hash: &str) -> Result<String, String> {
    let js = format!(
        "node = rc.ellipse({}, {}, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width / 2,
        shape.height / 2,
        shape.width,
        shape.height,
        shape.stroke_width,
        BASE_ROUGH_PROPS
    );
    let paths = compute_rough_path_data(runner, &js)?;
    let (fill, stroke) = shape_theme_fill_stroke(shape);

    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    path_el.fill = fill.clone();
    path_el.stroke = stroke.clone();
    path_el.fill_pattern = shape.fill_pattern.clone();
    path_el.class_name = "shape".to_owned();
    path_el.style = shape_css_style(shape);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            path_el.mask = format!("url(#{diagram_hash})");
        }
    }
    let mut out = String::new();
    for p in paths {
        path_el.d = p;
        out.push_str(&path_el.render());
    }

    let mut so = ThemableElement::new("ellipse", None);
    so.set_translate(
        (shape.pos.x + shape.width / 2) as f64,
        (shape.pos.y + shape.height / 2) as f64,
    );
    so.rx = Some((shape.width / 2) as f64);
    so.ry = Some((shape.height / 2) as f64);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            so.mask = format!("url(#{diagram_hash})");
        }
    }
    out.push_str(&render_sketch_overlay(&mut so, &fill)?);
    Ok(out)
}

/// `d2sketch.DoubleOval`.
pub fn double_oval(
    runner: &SketchRunner,
    shape: &Shape,
    diagram_hash: &str,
) -> Result<String, String> {
    let big = format!(
        "node = rc.ellipse({}, {}, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width / 2,
        shape.height / 2,
        shape.width,
        shape.height,
        shape.stroke_width,
        BASE_ROUGH_PROPS
    );
    let small = format!(
        "node = rc.ellipse({}, {}, {}, {}, {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
        shape.width / 2,
        shape.height / 2,
        shape.width - INNER_BORDER_OFFSET * 2,
        shape.height - INNER_BORDER_OFFSET * 2,
        shape.stroke_width,
        BASE_ROUGH_PROPS
    );
    let big_paths = compute_rough_path_data(runner, &big)?;
    let small_paths = compute_rough_path_data(runner, &small)?;

    let (fill, stroke) = shape_theme_fill_stroke(shape);
    let mut out = String::new();
    let mut path_el = ThemableElement::new("path", None);
    path_el.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    path_el.fill = fill.clone();
    path_el.stroke = stroke.clone();
    path_el.fill_pattern = shape.fill_pattern.clone();
    path_el.class_name = "shape".to_owned();
    path_el.style = shape_css_style(shape);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            path_el.mask = format!("url(#{diagram_hash})");
        }
    }
    for p in big_paths {
        path_el.d = p;
        out.push_str(&path_el.render());
    }

    let mut inner = ThemableElement::new("path", None);
    inner.set_translate(shape.pos.x as f64, shape.pos.y as f64);
    inner.fill = "transparent".to_owned();
    inner.stroke = stroke.clone();
    inner.class_name = "shape".to_owned();
    inner.style = shape_css_style(shape);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            inner.mask = format!("url(#{diagram_hash})");
        }
    }
    for p in small_paths {
        inner.d = p;
        out.push_str(&inner.render());
    }

    let mut so = ThemableElement::new("ellipse", None);
    so.set_translate(
        (shape.pos.x + shape.width / 2) as f64,
        (shape.pos.y + shape.height / 2) as f64,
    );
    so.rx = Some((shape.width / 2) as f64);
    so.ry = Some((shape.height / 2) as f64);
    if !shape.text.label.is_empty() {
        let lp = d2_label::Position::from_string(&shape.label_position);
        if lp.is_border() {
            so.mask = format!("url(#{diagram_hash})");
        }
    }
    out.push_str(&render_sketch_overlay(&mut so, &shape.fill)?);
    Ok(out)
}

/// `d2sketch.Paths`: rough each SVG path independently + per-path overlay.
pub fn paths(
    runner: &SketchRunner,
    shape: &Shape,
    diagram_hash: &str,
    paths_in: &[String],
) -> Result<String, String> {
    let mut out = String::new();
    for path in paths_in {
        let js = format!(
            "node = rc.path(\"{}\", {{\n\t\t\tfill: \"#000\",\n\t\t\tstroke: \"#000\",\n\t\t\tstrokeWidth: {},\n\t\t\t{}\n\t\t}});",
            path, shape.stroke_width, BASE_ROUGH_PROPS
        );
        let sketch_paths = compute_rough_path_data(runner, &js)?;
        let (fill, stroke) = shape_theme_fill_stroke(shape);
        let mut path_el = ThemableElement::new("path", None);
        path_el.fill = fill.clone();
        path_el.stroke = stroke;
        path_el.fill_pattern = shape.fill_pattern.clone();
        path_el.class_name = "shape".to_owned();
        path_el.style = shape_css_style(shape);
        if !shape.text.label.is_empty() {
            let lp = d2_label::Position::from_string(&shape.label_position);
            if lp.is_border() {
                path_el.mask = format!("url(#{diagram_hash})");
            }
        }
        for p in &sketch_paths {
            path_el.d = p.clone();
            out.push_str(&path_el.render());
        }

        // One sketch-overlay per path, re-using pathEl's fill.
        let mut so = ThemableElement::new("path", None);
        if !shape.text.label.is_empty() {
            let lp = d2_label::Position::from_string(&shape.label_position);
            if lp.is_border() {
                so.mask = format!("url(#{diagram_hash})");
            }
        }
        for p in sketch_paths {
            so.d = p;
            out.push_str(&render_sketch_overlay(&mut so, &fill)?);
            // WARNING: sketch_overlay mutates class_name by appending
            // " sketch-overlay-X".  Before reusing, reset.
            so.class_name.clear();
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// `d2sketch.Connection`: render the connection stroke (animated or not).
/// Called with the already-computed SVG path and the `attrs` string (Go
/// passes `mask="url(#diagramHash)"` for masking).
pub fn connection(
    runner: &SketchRunner,
    connection: &Connection,
    path: &str,
    attrs: &str,
) -> Result<String, String> {
    let animated_class = if connection.animated { " animated-connection" } else { "" };

    if connection.animated {
        // Match Go: bidirectional or absent arrows → split path into two halves
        // with reverse animation direction.  Otherwise emit a single sketched path.
        let bidirectional = (connection.dst_arrow == Arrowhead::None
            && connection.src_arrow == Arrowhead::None)
            || (connection.dst_arrow != Arrowhead::None
                && connection.src_arrow != Arrowhead::None);

        if bidirectional {
            let (p1, p2) = d2_svg_path::split_path(path, 0.5)
                .map_err(|e| format!("split_path: {e}"))?;

            let mut el1 = ThemableElement::new("path", None);
            el1.d = p1;
            el1.fill = "none".to_owned();
            el1.stroke = connection.stroke.clone();
            el1.class_name = format!("connection{}", animated_class);
            el1.style = connection_css_style(connection);
            el1.style.push_str("animation-direction: reverse;");
            el1.attributes = attrs.to_owned();

            let mut el2 = ThemableElement::new("path", None);
            el2.d = p2;
            el2.fill = "none".to_owned();
            el2.stroke = connection.stroke.clone();
            el2.class_name = format!("connection{}", animated_class);
            el2.style = connection_css_style(connection);
            el2.attributes = attrs.to_owned();

            return Ok(format!("{} {}", el1.render(), el2.render()));
        }

        let mut el = ThemableElement::new("path", None);
        el.d = path.to_owned();
        el.fill = "none".to_owned();
        el.stroke = connection.stroke.clone();
        el.class_name = format!("connection{}", animated_class);
        el.style = connection_css_style(connection);
        el.attributes = attrs.to_owned();
        return Ok(el.render());
    }

    // Non-animated: sketch the path via rough.js with roughness=0.5.
    //
    // Go: `fmt.Sprintf("node = rc.path(\"%s\", {roughness: %f, seed: 1});",
    // path, 0.5)` which prints `0.500000`.
    let js = format!(
        "node = rc.path(\"{}\", {{roughness: {:.6}, seed: 1}});",
        path, 0.5_f64
    );
    let sketch_paths = compute_rough_path_data(runner, &js)?;

    let mut out = String::new();
    let mut el = ThemableElement::new("path", None);
    el.fill = "none".to_owned();
    el.stroke = connection.stroke.clone();
    el.class_name = format!("connection{}", animated_class);
    el.style = connection_css_style(connection);
    el.attributes = attrs.to_owned();
    for p in sketch_paths {
        el.d = p;
        out.push_str(&el.render());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Arrowheads
// ---------------------------------------------------------------------------

/// Return the `(arrowJS, extraJS)` snippet pair for `arrowhead`.  Mirrors Go
/// `ArrowheadJS` exactly — seeds, fillStyle, and coordinate arrays must be
/// identical to produce byte-equivalent rough output.
fn arrowhead_js(arrowhead: Arrowhead, stroke: &str, stroke_width: i32) -> (String, String) {
    const BG_COLOR: &str = d2_color::N7;
    match arrowhead {
        Arrowhead::Arrow => (
            format!(
                "node = rc.linearPath([[-10, -4], [0, 0], [-10, 4]], {{ strokeWidth: {}, stroke: \"{}\", seed: 3 }})",
                stroke_width, stroke
            ),
            String::new(),
        ),
        Arrowhead::Triangle => (
            format!(
                "node = rc.polygon([[-10, -4], [0, 0], [-10, 4]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", seed: 2 }})",
                stroke_width, stroke, stroke
            ),
            String::new(),
        ),
        Arrowhead::UnfilledTriangle => (
            format!(
                "node = rc.polygon([[-10, -4], [0, 0], [-10, 4]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", seed: 2 }})",
                stroke_width, stroke, BG_COLOR
            ),
            String::new(),
        ),
        Arrowhead::Diamond => (
            format!(
                "node = rc.polygon([[-20, 0], [-10, 5], [0, 0], [-10, -5], [-20, 0]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", seed: 1 }})",
                stroke_width, stroke, BG_COLOR
            ),
            String::new(),
        ),
        Arrowhead::FilledDiamond => (
            format!(
                "node = rc.polygon([[-20, 0], [-10, 5], [0, 0], [-10, -5], [-20, 0]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"zigzag\", fillWeight: 4, seed: 1 }})",
                stroke_width, stroke, stroke
            ),
            String::new(),
        ),
        Arrowhead::Cross => (
            format!(
                "node = rc.linearPath([[-6, -6], [6, 6], [0, 0], [-6, 6], [0, 0], [6, -6]], {{ strokeWidth: {}, stroke: \"{}\", seed: 3 }})",
                stroke_width, stroke
            ),
            String::new(),
        ),
        Arrowhead::CfManyRequired => (
            format!(
                "node = rc.path(\"M-15,-10 -15,10 M0,10 -15,0 M0,-10 -15,0\", {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 4, seed: 2 }})",
                stroke_width, stroke, stroke
            ),
            String::new(),
        ),
        Arrowhead::CfMany => (
            format!(
                "node = rc.path(\"M0,10 -15,0 M0,-10 -15,0\", {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 4, seed: 8 }})",
                stroke_width, stroke, stroke
            ),
            format!(
                "node = rc.circle(-20, 0, 8, {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 1, seed: 4 }})",
                stroke_width, stroke, BG_COLOR
            ),
        ),
        Arrowhead::CfOneRequired => (
            format!(
                "node = rc.path(\"M-15,-10 -15,10 M-10,-10 -10,10\", {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 4, seed: 2 }})",
                stroke_width, stroke, stroke
            ),
            String::new(),
        ),
        Arrowhead::CfOne => (
            format!(
                "node = rc.path(\"M-10,-10 -10,10\", {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 4, seed: 3 }})",
                stroke_width, stroke, stroke
            ),
            format!(
                "node = rc.circle(-20, 0, 8, {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 1, seed: 5 }})",
                stroke_width, stroke, BG_COLOR
            ),
        ),
        Arrowhead::Circle => (
            format!(
                "node = rc.circle(-2, -1, 8, {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", fillWeight: 1, seed: 5 }})",
                stroke_width, stroke, BG_COLOR
            ),
            String::new(),
        ),
        Arrowhead::Box_ => (
            format!(
                "node = rc.polygon([[0, -10], [0, 10], [-20, 10], [-20, -10]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", seed: 1}})",
                stroke_width, stroke, BG_COLOR
            ),
            String::new(),
        ),
        Arrowhead::FilledBox => (
            format!(
                "node = rc.polygon([[0, -10], [0, 10], [-20, 10], [-20, -10]], {{ strokeWidth: {}, stroke: \"{}\", fill: \"{}\", fillStyle: \"solid\", seed: 1}})",
                stroke_width, stroke, stroke
            ),
            String::new(),
        ),
        // None / FilledCircle / Line aren't handled in Go (silently produce "").
        _ => (String::new(), String::new()),
    }
}

/// Render Go-style `%v` for floats: if the value is integer-valued, print
/// without decimals; otherwise print the shortest form that round-trips
/// (matches `strconv.FormatFloat(f, 'g', -1, 64)`).
fn fmt_v(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "+Inf".to_owned() } else { "-Inf".to_owned() };
    }
    // Go %v prints 0 for both 0 and -0 via default formatting? Actually it
    // does keep -0 as "-0".  Handle explicitly.
    if f == 0.0 {
        return if f.is_sign_negative() { "-0".to_owned() } else { "0".to_owned() };
    }
    // Ryu-like shortest round-trip. Rust's default `{f}` already produces the
    // shortest round-trippable representation, matching Go's `%v` for `float64`
    // in almost all cases.  One caveat: Go `%v` uses `-180` (no decimal) when
    // the float is an integer value; Rust's default `Debug` prints `-180` too.
    let s = format!("{f}");
    s
}

/// Render the source + destination arrowheads, mirroring Go `Arrowheads`.
pub fn arrowheads(
    runner: &SketchRunner,
    connection: &Connection,
    src_adj: &Point,
    dst_adj: &Point,
) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();

    if connection.src_arrow != Arrowhead::None {
        let (arrow_js, extra_js) =
            arrowhead_js(connection.src_arrow.clone(), &connection.stroke, connection.stroke_width);
        if arrow_js.is_empty() {
            return Ok(String::new());
        }
        let route = &connection.route;
        if route.len() < 2 {
            return Ok(String::new());
        }
        let starting = Segment::new(route[0], route[1]);
        let starting_vec = starting.to_vector().reverse();
        let angle = starting_vec.degrees();
        let transform = format!(
            "transform=\"translate({:.6} {:.6}) rotate({})\"",
            starting.start.x + src_adj.x,
            starting.start.y + src_adj.y,
            fmt_v(angle)
        );
        let mut rp = compute_rough_paths(runner, &arrow_js)?;
        if !extra_js.is_empty() {
            let mut extra = compute_rough_paths(runner, &extra_js)?;
            rp.append(&mut extra);
        }
        let mut path_el = ThemableElement::new("path", None);
        path_el.class_name = "connection".to_owned();
        path_el.attributes = transform;
        for r in rp {
            path_el.d = r.attrs.d.clone();
            path_el.fill = r.style.fill.clone();
            path_el.stroke = r.style.stroke.clone();
            path_el.style = r.style_css();
            parts.push(path_el.render());
        }
    }

    if connection.dst_arrow != Arrowhead::None {
        let (arrow_js, extra_js) =
            arrowhead_js(connection.dst_arrow.clone(), &connection.stroke, connection.stroke_width);
        if arrow_js.is_empty() {
            return Ok(String::new());
        }
        let route = &connection.route;
        if route.len() < 2 {
            return Ok(String::new());
        }
        let last = route.len() - 1;
        let ending = Segment::new(route[last - 1], route[last]);
        let ending_vec = ending.to_vector();
        let angle = ending_vec.degrees();
        let transform = format!(
            "transform=\"translate({:.6} {:.6}) rotate({})\"",
            ending.end.x + dst_adj.x,
            ending.end.y + dst_adj.y,
            fmt_v(angle)
        );
        let mut rp = compute_rough_paths(runner, &arrow_js)?;
        if !extra_js.is_empty() {
            let mut extra = compute_rough_paths(runner, &extra_js)?;
            rp.append(&mut extra);
        }
        let mut path_el = ThemableElement::new("path", None);
        path_el.class_name = "connection".to_owned();
        path_el.attributes = transform;
        for r in rp {
            path_el.d = r.attrs.d.clone();
            path_el.fill = r.style.fill.clone();
            path_el.stroke = r.style.stroke.clone();
            path_el.style = r.style_css();
            parts.push(path_el.render());
        }
    }

    Ok(parts.join(" "))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_floats_basic() {
        assert_eq!(truncate_floats("12.3456789"), "12.345678");
        assert_eq!(truncate_floats("0.1"), "0.1");
        assert_eq!(truncate_floats("M 10.12345678 20.1"), "M 10.123456 20.1");
        assert_eq!(truncate_floats("L -5.999999999"), "L -5.999999");
        assert_eq!(truncate_floats("no numbers"), "no numbers");
        assert_eq!(truncate_floats("1.123456789,2.987654321"), "1.123456,2.987654");
    }

    #[test]
    fn runner_round_trips_basic_rect() {
        let runner = SketchRunner::new().unwrap();
        runner.eval_unit("node = rc.rectangle(0, 0, 10, 10, {seed: 1})").unwrap();
        let json = runner.eval_string("JSON.stringify(node.children, null, '  ')").unwrap();
        assert!(json.contains("\"d\""));
    }

    #[test]
    fn fmt_v_matches_go() {
        assert_eq!(fmt_v(45.0), "45");
        assert_eq!(fmt_v(-90.0), "-90");
        // When computed via (atan2 as f32 as f64) * 180 / PI, the resulting
        // float64 has a bit pattern both Go and Rust serialize as
        // "90.00000250447816" — the bit-exact canonical form.
        let y = -86.0_f64;
        let x = 0.0_f64;
        let deg = ((y.atan2(x) as f32) as f64) * 180.0 / std::f64::consts::PI;
        assert_eq!(fmt_v(deg), "-90.00000250447816");
    }
}
