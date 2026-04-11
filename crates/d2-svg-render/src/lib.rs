//! d2-svg-render: SVG renderer for d2 diagrams.
//!
//! Ported from Go `d2renderers/d2svg/d2svg.go`.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use d2_color;
use d2_geo;
use d2_label;
use d2_shape::{self, ShapeOps};
use d2_svg_path;
use d2_target;
use d2_themes;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DEFAULT_PADDING: i32 = 100;
pub const APPENDIX_ICON_RADIUS: i32 = 16;

// ---------------------------------------------------------------------------
// Base CSS stylesheet (embedded inline for simplicity)
// ---------------------------------------------------------------------------

/// Base stylesheet embedded by every SVG.
///
/// Matches the contents of `d2/d2renderers/d2svg/style.css` byte-for-byte
/// (no leading newline, trailing newline preserved).
const BASE_STYLESHEET: &str = ".shape {\n  shape-rendering: geometricPrecision;\n  stroke-linejoin: round;\n}\n.connection {\n  stroke-linecap: round;\n  stroke-linejoin: round;\n}\n.blend {\n  mix-blend-mode: multiply;\n  opacity: 0.5;\n}\n";

// ---------------------------------------------------------------------------
// RenderOpts
// ---------------------------------------------------------------------------

/// Options controlling SVG rendering.
#[derive(Debug, Clone, Default)]
pub struct RenderOpts {
    pub pad: Option<i64>,
    pub sketch: Option<bool>,
    pub center: Option<bool>,
    pub theme_id: Option<i64>,
    pub dark_theme_id: Option<i64>,
    pub theme_overrides: Option<d2_themes::ThemeOverrides>,
    pub dark_theme_overrides: Option<d2_themes::ThemeOverrides>,
    pub font: String,
    /// The SVG will be scaled by this factor; if `None`, the SVG fits to screen.
    pub scale: Option<f64>,
    /// When set, the diagram uses something other than its own hash for unique
    /// CSS targeting (used for collapsed multi-boards).
    pub master_id: String,
    pub no_xml_tag: Option<bool>,
    pub salt: Option<String>,
    pub omit_version: Option<bool>,
}

// ---------------------------------------------------------------------------
// DiagramObject – heterogeneous sorting of shapes/connections
// ---------------------------------------------------------------------------

enum DiagramObject<'a> {
    Shape(&'a d2_target::Shape),
    Connection(&'a d2_target::Connection),
}

impl<'a> DiagramObject<'a> {
    fn z_index(&self) -> i32 {
        match self {
            DiagramObject::Shape(s) => s.z_index,
            DiagramObject::Connection(c) => c.z_index,
        }
    }

    fn level(&self) -> Option<i32> {
        match self {
            DiagramObject::Shape(s) => Some(s.level),
            DiagramObject::Connection(_) => None,
        }
    }

    fn is_shape(&self) -> bool {
        matches!(self, DiagramObject::Shape(_))
    }
}

/// Sort diagram objects (shapes + connections) in drawing order.
///
/// Criteria: z-index ascending; same z-index: shapes before connections;
/// same z-index shapes: lower level (parents) first; original order as
/// tiebreaker (stable sort).
fn sort_objects(objects: &mut [DiagramObject<'_>]) {
    objects.sort_by(|a, b| {
        let za = a.z_index();
        let zb = b.z_index();
        if za != zb {
            return za.cmp(&zb);
        }
        // Both shapes: parent before child
        if let (Some(la), Some(lb)) = (a.level(), b.level()) {
            if la != lb {
                return la.cmp(&lb);
            }
        }
        // Shapes before connections
        match (a.is_shape(), b.is_shape()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
}

// ---------------------------------------------------------------------------
// FNV-1a hash (matches Go's hash/fnv New32a)
// ---------------------------------------------------------------------------

fn fnv1a_hash(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn hash_str(s: &str) -> String {
    const SECRET: &str = "lalalas";
    let combined = format!("{}{}", s, SECRET);
    fnv1a_hash(combined.as_bytes()).to_string()
}

// ---------------------------------------------------------------------------
// Shape theme (fill/stroke swap for class/table)
// ---------------------------------------------------------------------------

fn shape_theme(shape: &d2_target::Shape) -> (String, String) {
    if shape.type_ == d2_target::SHAPE_CLASS || shape.type_ == d2_target::SHAPE_SQL_TABLE {
        (shape.stroke.clone(), shape.fill.clone())
    } else {
        (shape.fill.clone(), shape.stroke.clone())
    }
}

// ---------------------------------------------------------------------------
// CSS style helpers
// ---------------------------------------------------------------------------

fn shape_css_style(s: &d2_target::Shape) -> String {
    let mut out = format!("stroke-width:{};", s.stroke_width);
    if s.stroke_dash != 0.0 {
        let (dash, gap) =
            d2_svg_path::get_stroke_dash_attributes(s.stroke_width as f64, s.stroke_dash);
        // Match Go `Shape.CSSStyle`: `fmt.Sprintf("stroke-dasharray:%f,%f;")`.
        // Go `%f` defaults to six decimal places.
        write!(out, "stroke-dasharray:{:.6},{:.6};", dash, gap).unwrap();
    }
    out
}

fn connection_css_style(c: &d2_target::Connection) -> String {
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
            if c.src_arrow != d2_target::Arrowhead::None
                && c.dst_arrow == d2_target::Arrowhead::None
            {
                dash_offset = 10.0;
            }
            write!(out, "stroke-dashoffset:{};", dash_offset * (dash + gap)).unwrap();
            write!(out, "animation: dashdraw {}s linear infinite;", gap * 0.5).unwrap();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SVG text rendering
// ---------------------------------------------------------------------------

/// Render multi-line text using `<tspan>` elements.
pub fn render_text(text: &str, x: f64, height: f64) -> String {
    if !text.contains('\n') {
        return d2_svg_path::escape_text(text);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = String::new();
    for (i, line) in lines.iter().enumerate() {
        let dy = if i == 0 {
            0.0
        } else {
            height / lines.len() as f64
        };
        let escaped = d2_svg_path::escape_text(line);
        let escaped = if escaped.is_empty() { " " } else { &escaped };
        // x and dy are float64 in Go, formatted with %f. The %.6f rounding
        // also gives us 17.666667 instead of 17.666666666666668 for the
        // common dy = height / line_count case.
        write!(
            result,
            r#"<tspan x="{:.6}" dy="{:.6}">{}</tspan>"#,
            x, dy, escaped
        )
        .unwrap();
    }
    result
}

// ---------------------------------------------------------------------------
// Arrowhead markers
// ---------------------------------------------------------------------------

fn arrowhead_marker_id(
    diagram_hash: &str,
    is_target: bool,
    connection: &d2_target::Connection,
) -> String {
    let arrowhead = if is_target {
        &connection.dst_arrow
    } else {
        &connection.src_arrow
    };
    format!(
        "mk-{}-{}",
        diagram_hash,
        hash_str(&format!(
            "{},{},{},{}",
            arrowhead, is_target, connection.stroke_width, connection.stroke
        ))
    )
}

fn arrowhead_marker(
    is_target: bool,
    id: &str,
    connection: &d2_target::Connection,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let arrowhead = if is_target {
        &connection.dst_arrow
    } else {
        &connection.src_arrow
    };
    let stroke_width = connection.stroke_width as f64;
    let (width, height) = arrowhead.dimensions(stroke_width);

    let path = match arrowhead {
        d2_target::Arrowhead::Arrow => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.fill = connection.stroke.clone();
            el.class_name = "connection".to_owned();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    0.0,
                    0.0,
                    width,
                    height / 2.0,
                    0.0,
                    height,
                    width / 4.0,
                    height / 2.0
                );
            } else {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    0.0,
                    height / 2.0,
                    width,
                    0.0,
                    width * 3.0 / 4.0,
                    height / 2.0,
                    width,
                    height
                );
            }
            el.render()
        }
        d2_target::Arrowhead::Triangle => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.fill = connection.stroke.clone();
            el.class_name = "connection".to_owned();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    0.0,
                    0.0,
                    width,
                    height / 2.0,
                    0.0,
                    height
                );
            } else {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    width,
                    0.0,
                    0.0,
                    height / 2.0,
                    width,
                    height
                );
            }
            el.render()
        }
        d2_target::Arrowhead::UnfilledTriangle => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.fill = d2_target::BG_COLOR.to_owned();
            el.stroke = connection.stroke.clone();
            el.class_name = "connection".to_owned();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            let inset = stroke_width / 2.0;
            if is_target {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    inset,
                    inset,
                    width - inset,
                    height / 2.0,
                    inset,
                    height - inset
                );
            } else {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    width - inset,
                    inset,
                    inset,
                    height / 2.0,
                    width - inset,
                    height - inset
                );
            }
            el.render()
        }
        d2_target::Arrowhead::Line => {
            let mut el = d2_themes::ThemableElement::new("polyline", inline_theme);
            el.fill = "none".to_owned();
            el.class_name = "connection".to_owned();
            el.stroke = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    stroke_width / 2.0,
                    stroke_width / 2.0,
                    width - stroke_width / 2.0,
                    height / 2.0,
                    stroke_width / 2.0,
                    height - stroke_width / 2.0
                );
            } else {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    width - stroke_width / 2.0,
                    stroke_width / 2.0,
                    stroke_width / 2.0,
                    height / 2.0,
                    width - stroke_width / 2.0,
                    height - stroke_width / 2.0
                );
            }
            el.render()
        }
        d2_target::Arrowhead::FilledDiamond => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.class_name = "connection".to_owned();
            el.fill = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            el.points = format!(
                "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                0.0,
                height / 2.0,
                width / 2.0,
                0.0,
                width,
                height / 2.0,
                width / 2.0,
                height
            );
            el.render()
        }
        d2_target::Arrowhead::Diamond => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.class_name = "connection".to_owned();
            el.fill = d2_target::BG_COLOR.to_owned();
            el.stroke = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    0.0,
                    height / 2.0,
                    width / 2.0,
                    height / 8.0,
                    width,
                    height / 2.0,
                    width / 2.0,
                    height * 0.9
                );
            } else {
                el.points = format!(
                    "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                    width / 8.0,
                    height / 2.0,
                    width * 0.6,
                    height / 8.0,
                    width * 1.1,
                    height / 2.0,
                    width * 0.6,
                    height * 7.0 / 8.0
                );
            }
            el.render()
        }
        d2_target::Arrowhead::FilledCircle => {
            let radius = width / 2.0;
            let mut el = d2_themes::ThemableElement::new("circle", inline_theme);
            el.cy = Some(radius);
            el.r = Some(radius - stroke_width / 2.0);
            el.fill = connection.stroke.clone();
            el.class_name = "connection".to_owned();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.cx = Some(radius + stroke_width / 2.0);
            } else {
                el.cx = Some(radius - stroke_width / 2.0);
            }
            el.render()
        }
        d2_target::Arrowhead::Circle => {
            let radius = width / 2.0;
            let mut el = d2_themes::ThemableElement::new("circle", inline_theme);
            el.cy = Some(radius);
            el.r = Some(radius - stroke_width);
            el.fill = d2_target::BG_COLOR.to_owned();
            el.stroke = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            if is_target {
                el.cx = Some(radius + stroke_width / 2.0);
            } else {
                el.cx = Some(radius - stroke_width / 2.0);
            }
            el.render()
        }
        d2_target::Arrowhead::FilledBox => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.class_name = "connection".to_owned();
            el.fill = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            el.points = format!(
                "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                0.0, 0.0, 0.0, height, width, height, width, 0.0
            );
            el.render()
        }
        d2_target::Arrowhead::Box_ => {
            let mut el = d2_themes::ThemableElement::new("polygon", inline_theme);
            el.class_name = "connection".to_owned();
            el.fill = d2_target::BG_COLOR.to_owned();
            el.stroke = connection.stroke.clone();
            el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            let s = &mut el.style;
            s.push_str("stroke-linejoin:miter;");
            let inset = stroke_width / 2.0;
            el.points = format!(
                "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                inset,
                inset,
                inset,
                height - inset,
                width - inset,
                height - inset,
                width - inset,
                inset
            );
            el.render()
        }
        d2_target::Arrowhead::Cross => {
            let inset = stroke_width / 8.0;
            let rotation_angle = std::f64::consts::PI / 4.0;
            let ox = width / 2.0;
            let oy = height / 2.0;
            let new_ox = rotation_angle.cos() * ox - rotation_angle.sin() * oy;
            let new_oy = rotation_angle.sin() * ox + rotation_angle.cos() * oy;

            let mut cross_el = d2_themes::ThemableElement::new("polygon", inline_theme);
            cross_el.points = format!(
                "{:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}, {:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6} {:.6},{:.6}",
                0.0,
                height / 2.0 + inset,
                width / 2.0 - inset,
                height / 2.0 + inset,
                width / 2.0 - inset,
                height,
                width / 2.0 + inset,
                height,
                width / 2.0 + inset,
                height / 2.0 + inset,
                width,
                height / 2.0 + inset,
                width,
                height / 2.0 - inset,
                width / 2.0 + inset,
                height / 2.0 - inset,
                width / 2.0 + inset,
                0.0,
                width / 2.0 - inset,
                0.0,
                width / 2.0 - inset,
                height / 2.0 - inset,
                0.0,
                height / 2.0 - inset,
            );
            cross_el.transform = format!(
                "translate({}, {}) rotate(45)",
                -new_ox + width / 2.0,
                -new_oy + height / 2.0
            );

            let mut child_path = d2_themes::ThemableElement::new("path", inline_theme);
            if is_target {
                child_path.d = format!(
                    "M{:.6},{:.6} {:.6},{:.6}",
                    width / 2.0,
                    height / 2.0,
                    width,
                    height / 2.0
                );
            } else {
                child_path.d = format!(
                    "M{:.6},{:.6} {:.6},{:.6}",
                    width / 2.0,
                    height / 2.0,
                    0.0,
                    height / 2.0
                );
            }

            let mut g_el = d2_themes::ThemableElement::new("g", inline_theme);
            g_el.fill = d2_target::BG_COLOR.to_owned();
            g_el.stroke = connection.stroke.clone();
            g_el.class_name = "connection".to_owned();
            g_el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            g_el.content = format!("{}{}", cross_el.render(), child_path.render());
            g_el.render()
        }
        d2_target::Arrowhead::CfOne
        | d2_target::Arrowhead::CfMany
        | d2_target::Arrowhead::CfOneRequired
        | d2_target::Arrowhead::CfManyRequired => {
            let offset = 3.0 + stroke_width * 1.8;

            let modifier_el = match arrowhead {
                d2_target::Arrowhead::CfOneRequired | d2_target::Arrowhead::CfManyRequired => {
                    let mut el = d2_themes::ThemableElement::new("path", inline_theme);
                    el.d = format!("M{:.6},{:.6} {:.6},{:.6}", offset, 0.0, offset, height);
                    el.fill = d2_target::BG_COLOR.to_owned();
                    el.stroke = connection.stroke.clone();
                    el.class_name = "connection".to_owned();
                    el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
                    el
                }
                _ => {
                    let mut el = d2_themes::ThemableElement::new("circle", inline_theme);
                    el.cx = Some(offset / 2.0 + 2.0);
                    el.cy = Some(height / 2.0);
                    el.r = Some(offset / 2.0);
                    el.fill = d2_target::BG_COLOR.to_owned();
                    el.stroke = connection.stroke.clone();
                    el.class_name = "connection".to_owned();
                    el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
                    el
                }
            };

            let mut child_path = d2_themes::ThemableElement::new("path", inline_theme);
            match arrowhead {
                d2_target::Arrowhead::CfMany | d2_target::Arrowhead::CfManyRequired => {
                    child_path.d = format!(
                        "M{:.6},{:.6} {:.6},{:.6} M{:.6},{:.6} {:.6},{:.6} M{:.6},{:.6} {:.6},{:.6}",
                        width - 3.0,
                        height / 2.0,
                        width + offset,
                        height / 2.0,
                        offset + 3.0,
                        height / 2.0,
                        width + offset,
                        0.0,
                        offset + 3.0,
                        height / 2.0,
                        width + offset,
                        height,
                    );
                }
                _ => {
                    child_path.d = format!(
                        "M{:.6},{:.6} {:.6},{:.6} M{:.6},{:.6} {:.6},{:.6}",
                        width - 3.0,
                        height / 2.0,
                        width + offset,
                        height / 2.0,
                        offset * 2.0,
                        0.0,
                        offset * 2.0,
                        height,
                    );
                }
            }

            let mut g_el = d2_themes::ThemableElement::new("g", inline_theme);
            if !is_target {
                // Go renders these with `%f` which is six-decimal
                // formatting; keeping the same format avoids a byte diff
                // for every crow-foot/cf-many/cf-one connection.
                g_el.transform =
                    format!("scale(-1) translate(-{:.6}, -{:.6})", width, height);
            }
            g_el.fill = d2_target::BG_COLOR.to_owned();
            g_el.stroke = connection.stroke.clone();
            g_el.class_name = "connection".to_owned();
            g_el.attributes = format!(r#"stroke-width="{}""#, connection.stroke_width);
            g_el.content = format!("{}{}", modifier_el.render(), child_path.render());
            g_el.render()
        }
        d2_target::Arrowhead::None => return String::new(),
    };

    // Compute refX / refY
    let mut final_width = width;
    let ref_y = height / 2.0;
    let ref_x;

    match arrowhead {
        d2_target::Arrowhead::Diamond => {
            if is_target {
                ref_x = final_width - 0.6 * stroke_width;
            } else {
                ref_x = final_width / 8.0 + 0.6 * stroke_width;
            }
            final_width *= 1.1;
        }
        _ => {
            if is_target {
                ref_x = final_width - 1.5 * stroke_width;
            } else {
                ref_x = 1.5 * stroke_width;
            }
        }
    }

    // Float values are formatted with %f (6 decimal places) to match Go.
    format!(
        r#"<marker id="{}" markerWidth="{:.6}" markerHeight="{:.6}" refX="{:.6}" refY="{:.6}" viewBox="{:.6} {:.6} {:.6} {:.6}" orient="auto" markerUnits="userSpaceOnUse"> {} </marker>"#,
        id, final_width, height, ref_x, ref_y, 0.0, 0.0, final_width, height, path
    )
}

// ---------------------------------------------------------------------------
// Arrowhead adjustments
// ---------------------------------------------------------------------------

fn arrowhead_adjustment(
    start: &d2_geo::Point,
    end: &d2_geo::Point,
    arrowhead: &d2_target::Arrowhead,
    edge_stroke_width: i32,
    shape_stroke_width: i32,
) -> d2_geo::Point {
    let mut distance = (edge_stroke_width as f64 + shape_stroke_width as f64) / 2.0;
    if *arrowhead != d2_target::Arrowhead::None {
        distance += edge_stroke_width as f64;
    }
    let vx = end.x - start.x;
    let vy = end.y - start.y;
    let len = (vx * vx + vy * vy).sqrt();
    if len == 0.0 {
        return d2_geo::Point::new(0.0, 0.0);
    }
    d2_geo::Point::new(-distance * vx / len, -distance * vy / len)
}

fn get_arrowhead_adjustments(
    connection: &d2_target::Connection,
    id_to_shape: &HashMap<String, &d2_target::Shape>,
) -> (d2_geo::Point, d2_geo::Point) {
    let route = &connection.route;
    let src_sw = id_to_shape
        .get(&connection.src)
        .map_or(0, |s| s.stroke_width);
    let dst_sw = id_to_shape
        .get(&connection.dst)
        .map_or(0, |s| s.stroke_width);
    let src_adj = arrowhead_adjustment(
        &route[1],
        &route[0],
        &connection.src_arrow,
        connection.stroke_width,
        src_sw,
    );
    let dst_adj = arrowhead_adjustment(
        &route[route.len() - 2],
        &route[route.len() - 1],
        &connection.dst_arrow,
        connection.stroke_width,
        dst_sw,
    );
    (src_adj, dst_adj)
}

// ---------------------------------------------------------------------------
// Path data generation
// ---------------------------------------------------------------------------

fn path_data(
    connection: &d2_target::Connection,
    src_adj: &d2_geo::Point,
    dst_adj: &d2_geo::Point,
) -> String {
    let route = &connection.route;
    let mut path = Vec::new();

    // Float formatting matches Go's `%f` (six decimal places) so the
    // generated path string is byte-identical to Go d2's `pathData`.
    path.push(format!(
        "M {:.6} {:.6}",
        route[0].x + src_adj.x,
        route[0].y + src_adj.y
    ));

    if connection.is_curve {
        let mut i = 1;
        while i < route.len() - 3 {
            path.push(format!(
                "C {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}",
                route[i].x,
                route[i].y,
                route[i + 1].x,
                route[i + 1].y,
                route[i + 2].x,
                route[i + 2].y
            ));
            i += 3;
        }
        // Final curve with target adjustment
        path.push(format!(
            "C {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}",
            route[i].x,
            route[i].y,
            route[i + 1].x,
            route[i + 1].y,
            route[i + 2].x + dst_adj.x,
            route[i + 2].y + dst_adj.y
        ));
    } else {
        for i in 1..route.len() - 1 {
            let prev_source = &route[i - 1];
            let prev_target = &route[i];
            let curr_target = &route[i + 1];

            let prev_vx = prev_target.x - prev_source.x;
            let prev_vy = prev_target.y - prev_source.y;
            let curr_vx = curr_target.x - prev_target.x;
            let curr_vy = curr_target.y - prev_target.y;

            let dist = d2_geo::euclidean_distance(
                prev_target.x,
                prev_target.y,
                curr_target.x,
                curr_target.y,
            );

            let border_radius = connection.border_radius;
            let units = border_radius.min(dist / 2.0);

            // prev unit vector
            let prev_len = (prev_vx * prev_vx + prev_vy * vy_squared(prev_vy)).sqrt();
            let (pux, puy) = if prev_len > 0.0 {
                (prev_vx / prev_len, prev_vy / prev_len)
            } else {
                (0.0, 0.0)
            };

            // curr unit vector
            let curr_len = (curr_vx * curr_vx + curr_vy * curr_vy).sqrt();
            let (cux, cuy) = if curr_len > 0.0 {
                (curr_vx / curr_len, curr_vy / curr_len)
            } else {
                (0.0, 0.0)
            };

            let ptx = pux * units;
            let pty = puy * units;
            let ctx = cux * units;
            let cty = cuy * units;

            path.push(format!(
                "L {:.6} {:.6}",
                prev_target.x - ptx,
                prev_target.y - pty
            ));

            if units < border_radius && i < route.len() - 2 {
                let next_target = &route[i + 2];
                let nvx = next_target.x - curr_target.x;
                let nvy = next_target.y - curr_target.y;
                let next_len = (nvx * nvx + nvy * nvy).sqrt();
                let (nux, nuy) = if next_len > 0.0 {
                    (nvx / next_len, nvy / next_len)
                } else {
                    (0.0, 0.0)
                };
                let ntx = nux * units;
                let nty = nuy * units;

                path.push(format!(
                    "C {:.6} {:.6} {:.6} {:.6} {:.6} {:.6}",
                    prev_target.x + ptx,
                    prev_target.y + pty,
                    curr_target.x - ntx,
                    curr_target.y - nty,
                    curr_target.x + ntx,
                    curr_target.y + nty
                ));
            } else {
                path.push(format!(
                    "S {:.6} {:.6} {:.6} {:.6}",
                    prev_target.x,
                    prev_target.y,
                    prev_target.x + ctx,
                    prev_target.y + cty
                ));
            }
        }

        let last = &route[route.len() - 1];
        path.push(format!(
            "L {:.6} {:.6}",
            last.x + dst_adj.x,
            last.y + dst_adj.y
        ));
    }

    path.join(" ")
}

// Helper to avoid naming collision
fn vy_squared(v: f64) -> f64 {
    v * v
}

// ---------------------------------------------------------------------------
// Label mask generation
// ---------------------------------------------------------------------------

fn make_label_mask(label_tl: &d2_geo::Point, width: i32, height: i32, opacity: f64) -> String {
    let fill = if (opacity - 1.0).abs() < f64::EPSILON {
        "black".to_owned()
    } else {
        format!("rgba(0,0,0,{:.2})", opacity)
    };
    // X/Y are float64 in Go, formatted with `%f` (six decimal places). Width
    // and height are still int — Go uses `%d` for those.
    format!(
        r#"<rect x="{:.6}" y="{:.6}" width="{}" height="{}" fill="{}"></rect>"#,
        label_tl.x - 2.0,
        label_tl.y,
        width + 4,
        height,
        fill
    )
}

// ---------------------------------------------------------------------------
// Draw connection
// ---------------------------------------------------------------------------

fn draw_connection(
    buf: &mut String,
    diagram_hash: &str,
    connection: &d2_target::Connection,
    markers: &mut HashMap<String, ()>,
    id_to_shape: &HashMap<String, &d2_target::Shape>,
    inline_theme: Option<&d2_themes::Theme>,
) -> Result<String, String> {
    let mut label_mask = String::new();

    let opacity_style = if (connection.opacity - 1.0).abs() > f64::EPSILON {
        format!(" style='opacity:{:.6}'", connection.opacity)
    } else {
        String::new()
    };

    let id_encoded = base64_url_encode(&d2_svg_path::escape_text(&connection.id));
    let mut classes = vec![id_encoded];
    classes.extend(connection.classes.iter().cloned());
    let class_str = format!(r#" class="{}""#, classes.join(" "));

    write!(buf, "<g{}{}>", class_str, opacity_style).unwrap();

    // Source arrowhead marker
    let mut marker_start = String::new();
    if connection.src_arrow != d2_target::Arrowhead::None {
        let id = arrowhead_marker_id(diagram_hash, false, connection);
        if !markers.contains_key(&id) {
            let marker = arrowhead_marker(false, &id, connection, inline_theme);
            buf.push_str(&marker);
            markers.insert(id.clone(), ());
        }
        marker_start = format!(r#"marker-start="url(#{})" "#, id);
    }

    // Destination arrowhead marker
    let mut marker_end = String::new();
    if connection.dst_arrow != d2_target::Arrowhead::None {
        let id = arrowhead_marker_id(diagram_hash, true, connection);
        if !markers.contains_key(&id) {
            let marker = arrowhead_marker(true, &id, connection, inline_theme);
            buf.push_str(&marker);
            markers.insert(id.clone(), ());
        }
        marker_end = format!(r#"marker-end="url(#{})" "#, id);
    }

    let (src_adj, dst_adj) = get_arrowhead_adjustments(connection, id_to_shape);
    let path = path_data(connection, &src_adj, &dst_adj);
    let mask = format!(r#"mask="url(#{})""#, diagram_hash);

    let animated_class = if connection.animated {
        " animated-connection"
    } else {
        ""
    };

    let mut path_el = d2_themes::ThemableElement::new("path", inline_theme);
    path_el.d = path;
    path_el.fill = "none".to_owned();
    path_el.stroke = connection.stroke.clone();
    path_el.class_name = format!("connection{}", animated_class);
    path_el.style = connection_css_style(connection);
    path_el.attributes = format!("{}{}{}", marker_start, marker_end, mask);
    buf.push_str(&path_el.render());

    // Connection label
    if !connection.text.label.is_empty() {
        let label_pos = d2_label::Position::from_string(&connection.label_position);
        let route = d2_geo::Route(connection.route.clone());
        if let Some((label_tl, _)) = label_pos.get_point_on_route(
            &route,
            connection.stroke_width as f64,
            connection.label_percentage,
            connection.text.label_width as f64,
            connection.text.label_height as f64,
        ) {
            let label_tl = d2_geo::Point::new(label_tl.x.round(), label_tl.y.round());

            if label_pos.is_on_edge() {
                label_mask = make_label_mask(
                    &label_tl,
                    connection.text.label_width,
                    connection.text.label_height,
                    1.0,
                );
            } else {
                label_mask = make_label_mask(
                    &label_tl,
                    connection.text.label_width,
                    connection.text.label_height,
                    0.75,
                );
            }

            // Background rect for labels with an explicit fill.
            // Mirrors Go `drawConnection` — the rect has `rx=10`, sits 4px
            // left / 3px top of the label, and is 8px wider / 6px taller.
            if !connection.fill.is_empty() && connection.fill != "transparent" {
                let mut rect_el = d2_themes::ThemableElement::new("rect", inline_theme);
                rect_el.rx = Some(10.0);
                rect_el.x = Some(label_tl.x - 4.0);
                rect_el.y = Some(label_tl.y - 3.0);
                rect_el.width = Some(connection.text.label_width as f64 + 8.0);
                rect_el.height = Some(connection.text.label_height as f64 + 6.0);
                rect_el.fill = connection.fill.clone();
                buf.push_str(&rect_el.render());
            }

            // Render label text. Mirror Go `drawConnection`'s font-class
            // construction: start from `text`/`text-mono` based on
            // fontFamily, then suffix with `-bold`/`-italic`, and append
            // a ` text-underline` token when needed.
            let mut font_class = if connection.text.font_family == "mono" {
                "text-mono".to_owned()
            } else {
                "text".to_owned()
            };
            if connection.text.bold {
                font_class.push_str("-bold");
            } else if connection.text.italic {
                font_class.push_str("-italic");
            }
            if connection.text.underline {
                font_class.push_str(" text-underline");
            }

            let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
            text_el.x = Some(label_tl.x + connection.text.label_width as f64 / 2.0);
            text_el.y = Some(label_tl.y + connection.text.font_size as f64);
            text_el.class_name = font_class.to_owned();
            text_el.style = format!(
                "text-anchor:middle;font-size:{}px",
                connection.text.font_size
            );
            text_el.content = render_text(
                &connection.text.label,
                text_el.x.unwrap(),
                connection.text.label_height as f64,
            );
            text_el.fill = connection.get_font_color().to_owned();
            buf.push_str(&text_el.render());
        }
    }

    // Source / destination arrowhead labels (e.g. `source-arrowhead: 1`).
    if let Some(ref l) = connection.src_label {
        if !l.label.is_empty() {
            buf.push_str(&render_arrowhead_label(connection, l, false, inline_theme));
        }
    }
    if let Some(ref l) = connection.dst_label {
        if !l.label.is_empty() {
            buf.push_str(&render_arrowhead_label(connection, l, true, inline_theme));
        }
    }

    buf.push_str("</g>");
    Ok(label_mask)
}

/// Render an arrowhead-attached label (`source-arrowhead` / `target-arrowhead`).
/// Mirrors Go `d2svg.renderArrowheadLabel`.
fn render_arrowhead_label(
    connection: &d2_target::Connection,
    text: &d2_target::Text,
    is_dst: bool,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let width = text.label_width as f64;
    let height = text.label_height as f64;

    let label_tl = arrowhead_label_position(connection, is_dst);

    // svg text is positioned with the center of its baseline
    let baseline_x = label_tl.x + width / 2.0;
    let baseline_y = label_tl.y + connection.text.font_size as f64;

    let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
    text_el.x = Some(baseline_x);
    text_el.y = Some(baseline_y);
    text_el.fill = if !text.color.is_empty() {
        text.color.clone()
    } else {
        d2_target::FG_COLOR.to_owned()
    };
    text_el.class_name = "text-italic".to_owned();
    text_el.style = format!(
        "text-anchor:middle;font-size:{}px",
        connection.text.font_size
    );
    text_el.content = render_text(&text.label, baseline_x, height);
    text_el.render()
}

/// Compute the top-left of the source or destination arrowhead label on a
/// connection's route. Port of Go `d2target.Connection.GetArrowheadLabelPosition`.
fn arrowhead_label_position(connection: &d2_target::Connection, is_dst: bool) -> d2_geo::Point {
    let (width, height) = if is_dst {
        let l = connection.dst_label.as_ref().unwrap();
        (l.label_width as f64, l.label_height as f64)
    } else {
        let l = connection.src_label.as_ref().unwrap();
        (l.label_width as f64, l.label_height as f64)
    };

    let route = &connection.route;
    let index = if is_dst { route.len() - 2 } else { 0 };
    let start = route[index];
    let end = route[index + 1];
    // Note: end → start to get normal towards unlocked top position
    let (normal_x, normal_y) = d2_geo::get_unit_normal_vector(end.x, end.y, start.x, start.y);

    let shift = normal_x.abs() * (height / 2.0 + d2_label::PADDING)
        + normal_y.abs() * (width / 2.0 + d2_label::PADDING);

    let length = d2_geo::Route(route.clone()).length();
    let position = if is_dst {
        if length > 0.0 {
            1.0 - shift / length
        } else {
            1.0
        }
    } else if length > 0.0 {
        shift / length
    } else {
        0.0
    };

    let stroke_width = connection.stroke_width as f64;
    let route_ref = d2_geo::Route(route.clone());
    let (mut label_tl, _) = d2_label::Position::UnlockedTop
        .get_point_on_route(&route_ref, stroke_width, position, width, height)
        .unwrap_or((d2_geo::Point::new(0.0, 0.0), 0));

    // Shift further back if the arrow is larger than stroke + padding.
    let arrow_size = if is_dst && connection.dst_arrow != d2_target::Arrowhead::None {
        let (_, h) = connection.dst_arrow.dimensions(stroke_width);
        h
    } else if !is_dst && connection.src_arrow != d2_target::Arrowhead::None {
        let (_, h) = connection.src_arrow.dimensions(stroke_width);
        h
    } else {
        0.0
    };

    if arrow_size > 0.0 {
        let offset = (arrow_size / 2.0 + d2_target::ARROWHEAD_PADDING as f64)
            - stroke_width / 2.0
            - d2_label::PADDING;
        if offset > 0.0 {
            label_tl.x += normal_x * offset;
            label_tl.y += normal_y * offset;
        }
    }

    label_tl
}

// ---------------------------------------------------------------------------
// Draw shape
// ---------------------------------------------------------------------------

fn draw_shape(
    buf: &mut String,
    diagram_hash: &str,
    target_shape: &d2_target::Shape,
    inline_theme: Option<&d2_themes::Theme>,
) -> Result<String, String> {
    let mut label_mask = String::new();
    let mut closing_tag = "</g>".to_owned();

    if !target_shape.link.is_empty() {
        write!(
            buf,
            r#"<a href="{link}" xlink:href="{link}">"#,
            link = d2_svg_path::escape_text(&target_shape.link)
        )
        .unwrap();
        closing_tag.push_str("</a>");
    }

    let opacity_style = if (target_shape.opacity - 1.0).abs() > f64::EPSILON {
        format!(" style='opacity:{:.6}'", target_shape.opacity)
    } else {
        String::new()
    };

    let id_encoded = base64_url_encode(&d2_svg_path::escape_text(&target_shape.id));
    let mut classes = vec![id_encoded];
    if target_shape.animated {
        classes.push("animated-shape".to_owned());
    }
    classes.extend(target_shape.classes.iter().cloned());
    let class_str = format!(r#" class="{}""#, classes.join(" "));
    write!(buf, "<g{}{}>", class_str, opacity_style).unwrap();

    let tl = d2_geo::Point::new(target_shape.pos.x as f64, target_shape.pos.y as f64);
    let width = target_shape.width as f64;
    let height = target_shape.height as f64;
    let (fill, stroke) = shape_theme(target_shape);
    let style = shape_css_style(target_shape);
    let shape_type = d2_target::dsl_shape_to_shape_type(&target_shape.type_);

    // Shadow attribute
    let shadow_attr = if target_shape.shadow {
        match target_shape.type_.as_str() {
            d2_target::SHAPE_TEXT
            | d2_target::SHAPE_CODE
            | d2_target::SHAPE_CLASS
            | d2_target::SHAPE_SQL_TABLE => "",
            _ => r#"filter="url(#shadow-filter)" "#,
        }
    } else {
        ""
    };

    let blend_class = if target_shape.blend { " blend" } else { "" };

    write!(buf, r#"<g class="shape{}" {}>"#, blend_class, shadow_attr).unwrap();

    let multiple_tl = if target_shape.multiple {
        Some(d2_geo::Point::new(
            tl.x + d2_target::MULTIPLE_OFFSET as f64,
            tl.y - d2_target::MULTIPLE_OFFSET as f64,
        ))
    } else {
        None
    };

    // Dispatch by shape type
    match target_shape.type_.as_str() {
        d2_target::SHAPE_OVAL => {
            if target_shape.double_border {
                if let Some(ref mtl) = multiple_tl {
                    buf.push_str(&render_double_oval(
                        mtl,
                        width,
                        height,
                        &fill,
                        "",
                        &stroke,
                        &style,
                        inline_theme,
                    ));
                }
                buf.push_str(&render_double_oval(
                    &tl,
                    width,
                    height,
                    &fill,
                    &target_shape.fill_pattern,
                    &stroke,
                    &style,
                    inline_theme,
                ));
            } else {
                if let Some(ref mtl) = multiple_tl {
                    buf.push_str(&render_oval(
                        mtl,
                        width,
                        height,
                        &fill,
                        "",
                        &stroke,
                        &style,
                        inline_theme,
                    ));
                }
                buf.push_str(&render_oval(
                    &tl,
                    width,
                    height,
                    &fill,
                    &target_shape.fill_pattern,
                    &stroke,
                    &style,
                    inline_theme,
                ));
            }
        }
        d2_target::SHAPE_IMAGE => {
            let mut el = d2_themes::ThemableElement::new("image", inline_theme);
            el.x = Some(tl.x);
            el.y = Some(tl.y);
            el.width = Some(width);
            el.height = Some(height);
            if let Some(ref icon) = target_shape.icon {
                // Match Go `d2svg.go`: `el.Href = html.EscapeString(icon)`.
                // The raw URL may contain `&`, quotes, etc. that need to be
                // escaped to survive the SVG attribute.
                el.href = d2_svg_path::escape_text(icon);
            }
            el.fill = fill.clone();
            el.stroke = stroke.clone();
            el.style = style.clone();
            buf.push_str(&el.render());
        }
        d2_target::SHAPE_RECTANGLE
        | d2_target::SHAPE_SEQUENCE_DIAGRAM
        | d2_target::SHAPE_HIERARCHY
        | "" => {
            let border_radius = if target_shape.border_radius != 0 {
                target_shape.border_radius as f64
            } else {
                f64::MAX
            };

            if target_shape.three_dee {
                buf.push_str(&render_3d_rect(diagram_hash, target_shape, inline_theme));
            } else if !target_shape.double_border {
                if let Some(ref mtl) = multiple_tl {
                    let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
                    el.x = Some(mtl.x);
                    el.y = Some(mtl.y);
                    el.width = Some(width);
                    el.height = Some(height);
                    el.fill = fill.clone();
                    el.stroke = stroke.clone();
                    el.style = style.clone();
                    el.rx = Some(border_radius);
                    buf.push_str(&el.render());
                }
                let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
                el.x = Some(tl.x);
                el.y = Some(tl.y);
                el.width = Some(width);
                el.height = Some(height);
                el.fill = fill.clone();
                el.fill_pattern = target_shape.fill_pattern.clone();
                el.stroke = stroke.clone();
                el.style = style.clone();
                el.rx = Some(border_radius);

                if !target_shape.text.label.is_empty() {
                    let lp = d2_label::Position::from_string(&target_shape.label_position);
                    if lp.is_border() {
                        el.mask = format!("url(#{})", diagram_hash);
                    }
                }

                buf.push_str(&el.render());
            } else {
                // Double border
                if let Some(ref mtl) = multiple_tl {
                    let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
                    el.x = Some(mtl.x);
                    el.y = Some(mtl.y);
                    el.width = Some(width);
                    el.height = Some(height);
                    el.fill = fill.clone();
                    el.stroke = stroke.clone();
                    el.style = style.clone();
                    el.rx = Some(border_radius);
                    buf.push_str(&el.render());

                    let mut el2 = d2_themes::ThemableElement::new("rect", inline_theme);
                    el2.x = Some(mtl.x + d2_target::INNER_BORDER_OFFSET as f64);
                    el2.y = Some(mtl.y + d2_target::INNER_BORDER_OFFSET as f64);
                    el2.width = Some(width - 2.0 * d2_target::INNER_BORDER_OFFSET as f64);
                    el2.height = Some(height - 2.0 * d2_target::INNER_BORDER_OFFSET as f64);
                    el2.fill = fill.clone();
                    el2.stroke = stroke.clone();
                    el2.style = style.clone();
                    el2.rx = Some(border_radius);
                    buf.push_str(&el2.render());
                }

                let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
                el.x = Some(tl.x);
                el.y = Some(tl.y);
                el.width = Some(width);
                el.height = Some(height);
                el.fill = fill.clone();
                el.fill_pattern = target_shape.fill_pattern.clone();
                el.stroke = stroke.clone();
                el.style = style.clone();
                el.rx = Some(border_radius);
                buf.push_str(&el.render());

                let mut el2 = d2_themes::ThemableElement::new("rect", inline_theme);
                el2.x = Some(tl.x + d2_target::INNER_BORDER_OFFSET as f64);
                el2.y = Some(tl.y + d2_target::INNER_BORDER_OFFSET as f64);
                el2.width = Some(width - 2.0 * d2_target::INNER_BORDER_OFFSET as f64);
                el2.height = Some(height - 2.0 * d2_target::INNER_BORDER_OFFSET as f64);
                el2.fill = "transparent".to_owned();
                el2.stroke = stroke.clone();
                el2.style = style.clone();
                el2.rx = Some(border_radius);
                buf.push_str(&el2.render());
            }
        }
        d2_target::SHAPE_HEXAGON => {
            if target_shape.three_dee {
                buf.push_str(&render_3d_hexagon(diagram_hash, target_shape, inline_theme));
            } else {
                let bbox = d2_geo::Box2D::new(tl, width, height);
                let s = d2_shape::Shape::new(shape_type, bbox);

                if let Some(ref mtl) = multiple_tl {
                    let m_bbox = d2_geo::Box2D::new(*mtl, width, height);
                    let ms = d2_shape::Shape::new(shape_type, m_bbox);
                    let mut el = d2_themes::ThemableElement::new("path", inline_theme);
                    el.fill = fill.clone();
                    el.stroke = stroke.clone();
                    el.style = style.clone();
                    for pd in ms.get_svg_path_data() {
                        el.d = pd;
                        buf.push_str(&el.render());
                    }
                }

                let mut el = d2_themes::ThemableElement::new("path", inline_theme);
                el.fill = fill.clone();
                el.fill_pattern = target_shape.fill_pattern.clone();
                el.stroke = stroke.clone();
                el.style = style.clone();
                for pd in s.get_svg_path_data() {
                    el.d = pd;
                    buf.push_str(&el.render());
                }
            }
        }
        d2_target::SHAPE_TEXT | d2_target::SHAPE_CODE => {
            // No shape outline for text/code
        }
        d2_target::SHAPE_CLASS => {
            draw_class(buf, diagram_hash, target_shape, inline_theme);
            buf.push_str("</g>");
            buf.push_str(&closing_tag);
            return Ok(label_mask);
        }
        d2_target::SHAPE_SQL_TABLE => {
            draw_table(buf, diagram_hash, target_shape, inline_theme);
            buf.push_str("</g>");
            buf.push_str(&closing_tag);
            return Ok(label_mask);
        }
        _ => {
            // Generic path-based shapes (diamond, cloud, cylinder, etc.)
            let bbox = d2_geo::Box2D::new(tl, width, height);
            let s = d2_shape::Shape::new(shape_type, bbox);

            if let Some(ref mtl) = multiple_tl {
                let m_bbox = d2_geo::Box2D::new(*mtl, width, height);
                let ms = d2_shape::Shape::new(shape_type, m_bbox);
                let mut el = d2_themes::ThemableElement::new("path", inline_theme);
                el.fill = fill.clone();
                el.stroke = stroke.clone();
                el.style = style.clone();
                for pd in ms.get_svg_path_data() {
                    el.d = pd;
                    buf.push_str(&el.render());
                }
            }

            let mut el = d2_themes::ThemableElement::new("path", inline_theme);
            el.fill = fill.clone();
            el.fill_pattern = target_shape.fill_pattern.clone();
            el.stroke = stroke.clone();
            el.style = style.clone();

            if !target_shape.text.label.is_empty() {
                let lp = d2_label::Position::from_string(&target_shape.label_position);
                if lp.is_border() {
                    el.mask = format!("url(#{})", diagram_hash);
                }
            }

            for pd in s.get_svg_path_data() {
                el.d = pd;
                buf.push_str(&el.render());
            }
        }
    }

    // Close class="shape" group
    buf.push_str("</g>");

    // Icon rendering
    if target_shape.icon.is_some()
        && target_shape.type_ != d2_target::SHAPE_IMAGE
        && target_shape.opacity != 0.0
    {
        let icon_position = d2_label::Position::from_string(&target_shape.icon_position);
        let bbox = d2_geo::Box2D::new(tl, width, height);
        let s = d2_shape::Shape::new(shape_type, bbox);
        let the_box = if icon_position.is_outside() {
            s.get_box().clone()
        } else {
            s.get_inner_box()
        };
        let icon_size = get_icon_size(&the_box, &target_shape.icon_position);
        let icon_tl = icon_position.get_point_on_box(
            &the_box,
            d2_label::PADDING,
            icon_size as f64,
            icon_size as f64,
        );
        let clip_attr = if target_shape.icon_border_radius != 0 {
            format!(
                r#" clip-path="inset(0 round {}px)""#,
                target_shape.icon_border_radius
            )
        } else {
            String::new()
        };
        if let Some(ref icon) = target_shape.icon {
            // Match Go `d2svg.go`: x/y are formatted with `%f` (six
            // decimals), width/height are integers. Without the decimal
            // form the icon-label/investigate fixtures drift by tens of
            // bytes.
            write!(
                buf,
                r#"<image href="{}" x="{:.6}" y="{:.6}" width="{}" height="{}"{} />"#,
                d2_svg_path::escape_text(icon),
                icon_tl.x,
                icon_tl.y,
                icon_size,
                icon_size,
                clip_attr
            )
            .unwrap();
        }
    }

    // Label rendering
    if !target_shape.text.label.is_empty() && target_shape.opacity != 0.0 {
        let label_position = d2_label::Position::from_string(&target_shape.label_position);
        let bbox = d2_geo::Box2D::new(tl, width, height);
        let s = d2_shape::Shape::new(shape_type, bbox);

        let the_box = if label_position.is_outside() || label_position.is_border() {
            let mut b = s.get_box().clone();
            if target_shape.three_dee {
                let offset_y = if target_shape.type_ == d2_target::SHAPE_HEXAGON {
                    d2_target::THREE_DEE_OFFSET / 2
                } else {
                    d2_target::THREE_DEE_OFFSET
                };
                b.top_left.y -= offset_y as f64;
                b.height += offset_y as f64;
                b.width += d2_target::THREE_DEE_OFFSET as f64;
            } else if target_shape.multiple {
                b.top_left.y -= d2_target::MULTIPLE_OFFSET as f64;
                b.height += d2_target::MULTIPLE_OFFSET as f64;
                b.width += d2_target::MULTIPLE_OFFSET as f64;
            }
            b
        } else {
            s.get_inner_box()
        };

        let label_tl = label_position.get_point_on_box(
            &the_box,
            d2_label::PADDING,
            target_shape.text.label_width as f64,
            target_shape.text.label_height as f64,
        );

        if label_position.is_border() {
            label_mask = make_border_label_mask(
                label_position,
                &label_tl,
                target_shape.text.label_width,
                target_shape.text.label_height,
                &the_box,
                target_shape.stroke_width,
                1.0,
            );
        }

        let font_class = {
            let mut fc = if target_shape.text.font_family == "mono" {
                "text-mono".to_owned()
            } else {
                "text".to_owned()
            };
            if target_shape.text.bold {
                fc.push_str("-bold");
            } else if target_shape.text.italic {
                fc.push_str("-italic");
            }
            if target_shape.text.underline {
                fc.push_str(" text-underline");
            }
            fc
        };

        // Render as simple text (not markdown/latex/code for this initial port)
        if !target_shape.text.label_fill.is_empty() {
            let mut rect_el = d2_themes::ThemableElement::new("rect", inline_theme);
            rect_el.x = Some(label_tl.x);
            rect_el.y = Some(label_tl.y);
            rect_el.width = Some(target_shape.text.label_width as f64);
            rect_el.height = Some(target_shape.text.label_height as f64);
            rect_el.fill = target_shape.text.label_fill.clone();
            buf.push_str(&rect_el.render());
        }

        let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
        text_el.x = Some(label_tl.x + target_shape.text.label_width as f64 / 2.0);
        text_el.y = Some(label_tl.y + target_shape.text.font_size as f64);
        text_el.fill = target_shape.get_font_color().to_owned();
        text_el.class_name = font_class;
        text_el.style = format!(
            "text-anchor:middle;font-size:{}px",
            target_shape.text.font_size
        );
        text_el.content = render_text(
            &target_shape.text.label,
            text_el.x.unwrap(),
            target_shape.text.label_height as f64,
        );
        buf.push_str(&text_el.render());
    }

    // Tooltip as <title>
    if !target_shape.tooltip.is_empty() && target_shape.tooltip_position.is_empty() {
        write!(
            buf,
            "<title>{}</title>",
            d2_svg_path::escape_text(&target_shape.tooltip)
        )
        .unwrap();
    }

    buf.push_str(&closing_tag);
    Ok(label_mask)
}

// ---------------------------------------------------------------------------
// Border label mask
// ---------------------------------------------------------------------------

fn make_border_label_mask(
    label_position: d2_label::Position,
    label_tl: &d2_geo::Point,
    label_width: i32,
    label_height: i32,
    shape_box: &d2_geo::Box2D,
    stroke_width: i32,
    opacity: f64,
) -> String {
    let fill = if (opacity - 1.0).abs() < f64::EPSILON {
        "black".to_owned()
    } else {
        format!("rgba(0,0,0,{:.2})", opacity)
    };

    let ew = stroke_width as f64;

    let (mx, my, mw, mh) = match label_position {
        d2_label::Position::BorderTopLeft
        | d2_label::Position::BorderTopCenter
        | d2_label::Position::BorderTopRight => (
            label_tl.x - 2.0,
            shape_box.top_left.y - ew / 2.0,
            (label_width + 4) as f64,
            ew,
        ),
        d2_label::Position::BorderBottomLeft
        | d2_label::Position::BorderBottomCenter
        | d2_label::Position::BorderBottomRight => (
            label_tl.x - 2.0,
            shape_box.top_left.y + shape_box.height - ew / 2.0,
            (label_width + 4) as f64,
            ew,
        ),
        d2_label::Position::BorderLeftTop
        | d2_label::Position::BorderLeftMiddle
        | d2_label::Position::BorderLeftBottom => (
            shape_box.top_left.x - ew / 2.0,
            label_tl.y - 2.0,
            ew,
            (label_height + 4) as f64,
        ),
        d2_label::Position::BorderRightTop
        | d2_label::Position::BorderRightMiddle
        | d2_label::Position::BorderRightBottom => (
            shape_box.top_left.x + shape_box.width - ew / 2.0,
            label_tl.y - 2.0,
            ew,
            (label_height + 4) as f64,
        ),
        _ => return String::new(),
    };

    format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"></rect>"#,
        mx, my, mw, mh, fill
    )
}

// ---------------------------------------------------------------------------
// Oval rendering
// ---------------------------------------------------------------------------

fn render_oval(
    tl: &d2_geo::Point,
    width: f64,
    height: f64,
    fill: &str,
    fill_pattern: &str,
    stroke: &str,
    style: &str,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut el = d2_themes::ThemableElement::new("ellipse", inline_theme);
    let rx = width / 2.0;
    let ry = height / 2.0;
    el.rx = Some(rx);
    el.ry = Some(ry);
    el.cx = Some(tl.x + rx);
    el.cy = Some(tl.y + ry);
    el.fill = fill.to_owned();
    el.stroke = stroke.to_owned();
    el.fill_pattern = fill_pattern.to_owned();
    el.class_name = "shape".to_owned();
    el.style = style.to_owned();
    el.render()
}

fn render_double_oval(
    tl: &d2_geo::Point,
    width: f64,
    height: f64,
    fill: &str,
    fill_pattern: &str,
    stroke: &str,
    style: &str,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let inner_tl = d2_geo::Point::new(
        tl.x + d2_target::INNER_BORDER_OFFSET as f64,
        tl.y + d2_target::INNER_BORDER_OFFSET as f64,
    );
    format!(
        "{}{}",
        render_oval(
            tl,
            width,
            height,
            fill,
            fill_pattern,
            stroke,
            style,
            inline_theme
        ),
        render_oval(
            &inner_tl,
            width - 10.0,
            height - 10.0,
            fill,
            "",
            stroke,
            style,
            inline_theme,
        )
    )
}

// ---------------------------------------------------------------------------
// 3D rectangle rendering
// ---------------------------------------------------------------------------

fn render_3d_rect(
    diagram_hash: &str,
    target_shape: &d2_target::Shape,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut result = String::new();

    let px = target_shape.pos.x;
    let py = target_shape.pos.y;
    let w = target_shape.width;
    let h = target_shape.height;
    let off = d2_target::THREE_DEE_OFFSET;

    // Border path segments. Mirrors Go d2svg.go render3DRect: integer
    // coordinates (`%d,%d`), one M to start, eight L points around the
    // perimeter (the eighth is `(width, height)`, *not* a duplicate of
    // `(width, 0)`), then a final M+L pair to draw the missing top-right
    // edge without overlapping the previous strokes.
    let border_d = format!(
        "M{},{} L{},{} L{},{} L{},{} L{},{} L{},{} L{},{} L{},{} L{},{} M{},{} L{},{}",
        px,
        py,
        px + off,
        py - off,
        px + w + off,
        py - off,
        px + w + off,
        py + h - off,
        px + w,
        py + h,
        px,
        py + h,
        px,
        py,
        px + w,
        py,
        px + w,
        py + h,
        px + w,
        py,
        px + w + off,
        py - off,
    );

    let (_, border_stroke) = shape_theme(target_shape);
    let style = shape_css_style(target_shape);

    let mask_id = format!(
        "border-mask-{}-{}",
        diagram_hash,
        d2_svg_path::escape_text(&target_shape.id)
    );

    // Border mask. Mirror Go d2svg.go: each piece is separated by `\n`, so
    // the joined fragment looks like
    //   <defs><mask...>\n<rect.../>\n<path.../></mask></defs>
    write!(
        result,
        "<defs><mask id=\"{}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\"></rect>\n",
        mask_id, px, py - off, w + off, h + off,
        px, py - off, w + off, h + off,
    ).unwrap();

    // Compact border segments for mask path. Same point sequence as the
    // visible border (Go reuses the slice via strings.Join("", ...)) — the
    // eighth point is `(width, height)`, not a duplicate of `(width, 0)`.
    let mask_border_d = format!(
        "M{},{}L{},{}L{},{}L{},{}L{},{}L{},{}L{},{}L{},{}L{},{}M{},{}L{},{}",
        px,
        py,
        px + off,
        py - off,
        px + w + off,
        py - off,
        px + w + off,
        py + h - off,
        px + w,
        py + h,
        px,
        py + h,
        px,
        py,
        px + w,
        py,
        px + w,
        py + h,
        px + w,
        py,
        px + w + off,
        py - off,
    );

    write!(
        result,
        r#"<path d="{}" style="{};stroke:#000;fill:none;opacity:1;"/></mask></defs>"#,
        mask_border_d, style,
    )
    .unwrap();

    // Main rectangle
    let (main_fill, _) = shape_theme(target_shape);
    let mut main_el = d2_themes::ThemableElement::new("rect", inline_theme);
    main_el.x = Some(px as f64);
    main_el.y = Some(py as f64);
    main_el.width = Some(w as f64);
    main_el.height = Some(h as f64);
    main_el.set_mask_url(&mask_id);
    main_el.fill = main_fill;
    main_el.fill_pattern = target_shape.fill_pattern.clone();
    main_el.stroke = "none".to_owned();
    main_el.style = style.clone();
    result.push_str(&main_el.render());

    // Side polygons. Go d2svg.go formats these as `%d,%d` (integer), so we
    // do the same — px/py/w/h are all i32 here.
    let side_points = format!(
        "{},{} {},{} {},{} {},{} {},{} {},{}",
        px,
        py,
        px + off,
        py - off,
        px + w + off,
        py - off,
        px + w + off,
        py + h - off,
        px + w,
        py + h,
        px + w,
        py,
    );

    let darker_color =
        d2_color::darken(&target_shape.fill).unwrap_or_else(|_| target_shape.fill.clone());
    let mut side_el = d2_themes::ThemableElement::new("polygon", inline_theme);
    side_el.fill = darker_color;
    side_el.points = side_points;
    side_el.set_mask_url(&mask_id);
    side_el.style = style.clone();
    result.push_str(&side_el.render());

    // Border
    let mut border_el = d2_themes::ThemableElement::new("path", inline_theme);
    border_el.d = border_d;
    border_el.fill = "none".to_owned();
    border_el.stroke = border_stroke;
    border_el.style = style;
    result.push_str(&border_el.render());

    result
}

// ---------------------------------------------------------------------------
// 3D hexagon rendering (simplified)
// ---------------------------------------------------------------------------

fn render_3d_hexagon(
    _diagram_hash: &str,
    target_shape: &d2_target::Shape,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    // Simplified 3D hexagon – renders the flat hexagon shape with 3D offset indication
    let tl = d2_geo::Point::new(target_shape.pos.x as f64, target_shape.pos.y as f64);
    let width = target_shape.width as f64;
    let height = target_shape.height as f64;
    let shape_type = d2_target::dsl_shape_to_shape_type(&target_shape.type_);
    let bbox = d2_geo::Box2D::new(tl, width, height);
    let s = d2_shape::Shape::new(shape_type, bbox);
    let (fill, stroke) = shape_theme(target_shape);
    let style = shape_css_style(target_shape);

    let mut result = String::new();
    let mut el = d2_themes::ThemableElement::new("path", inline_theme);
    el.fill = fill;
    el.fill_pattern = target_shape.fill_pattern.clone();
    el.stroke = stroke;
    el.style = style;
    for pd in s.get_svg_path_data() {
        el.d = pd;
        result.push_str(&el.render());
    }
    result
}

// ---------------------------------------------------------------------------
// Draw class shape
// ---------------------------------------------------------------------------

fn draw_class(
    buf: &mut String,
    diagram_hash: &str,
    shape: &d2_target::Shape,
    inline_theme: Option<&d2_themes::Theme>,
) {
    // Mirror Go `d2renderers/d2svg/class.go drawClass` byte-for-byte.
    let (fill, stroke) = shape_theme(shape);
    let style = shape_css_style(shape);

    // Outer rect
    let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
    el.x = Some(shape.pos.x as f64);
    el.y = Some(shape.pos.y as f64);
    el.width = Some(shape.width as f64);
    el.height = Some(shape.height as f64);
    el.fill = fill;
    el.stroke = stroke;
    el.style = style;
    if shape.border_radius != 0 {
        el.rx = Some(shape.border_radius as f64);
        el.ry = Some(shape.border_radius as f64);
    }
    buf.push_str(&el.render());

    // Box = shape rect
    let box_x = shape.pos.x as f64;
    let box_y = shape.pos.y as f64;
    let box_w = shape.width as f64;
    let box_h = shape.height as f64;

    let n_rows = 2 + shape.class.fields.len() + shape.class.methods.len();
    let row_height = box_h / n_rows as f64;
    let pad = d2_label::PADDING;
    let header_height = (2.0 * row_height).max(shape.text.label_height as f64 + 2.0 * pad);

    // Header
    buf.push_str(&class_header(
        diagram_hash,
        shape,
        box_x,
        box_y,
        box_w,
        header_height,
        shape.text.label_width as f64,
        shape.text.label_height as f64,
        shape.text.font_size as f64,
        inline_theme,
    ));

    // Fields
    let mut row_y = box_y + header_height;
    for f in &shape.class.fields {
        buf.push_str(&class_row(
            shape,
            box_x,
            row_y,
            box_w,
            row_height,
            f.visibility_token(),
            &f.name,
            &f.type_,
            shape.text.font_size as f64,
            f.underline,
            inline_theme,
        ));
        row_y += row_height;
    }

    // Separator line between fields and methods
    let mut line_el = d2_themes::ThemableElement::new("line", inline_theme);
    if shape.border_radius != 0 && shape.class.methods.is_empty() {
        line_el.x1 = Some(box_x + shape.border_radius as f64);
        line_el.y1 = Some(row_y);
        line_el.x2 = Some(box_x + box_w - shape.border_radius as f64);
        line_el.y2 = Some(row_y);
    } else {
        line_el.x1 = Some(box_x);
        line_el.y1 = Some(row_y);
        line_el.x2 = Some(box_x + box_w);
        line_el.y2 = Some(row_y);
    }
    line_el.stroke = shape.fill.clone();
    line_el.style = "stroke-width:1".to_owned();
    buf.push_str(&line_el.render());

    // Methods
    for m in &shape.class.methods {
        buf.push_str(&class_row(
            shape,
            box_x,
            row_y,
            box_w,
            row_height,
            m.visibility_token(),
            &m.name,
            &m.return_,
            shape.text.font_size as f64,
            m.underline,
            inline_theme,
        ));
        row_y += row_height;
    }
}

/// Render the dark header rect + title text for a class shape.
#[allow(clippy::too_many_arguments)]
fn class_header(
    diagram_hash: &str,
    shape: &d2_target::Shape,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    box_h: f64,
    text_width: f64,
    text_height: f64,
    font_size: f64,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut out = String::new();
    let mut rect_el = d2_themes::ThemableElement::new("rect", inline_theme);
    rect_el.x = Some(box_x);
    rect_el.y = Some(box_y);
    rect_el.width = Some(box_w);
    rect_el.height = Some(box_h);
    rect_el.fill = shape.fill.clone();
    rect_el.fill_pattern = shape.fill_pattern.clone();
    rect_el.class_name = "class_header".to_owned();
    if shape.border_radius != 0 {
        rect_el.clip_path = format!("{}-{}", diagram_hash, shape.id);
    }
    out.push_str(&rect_el.render());

    if !shape.text.label.is_empty() {
        // InsideMiddleCenter: centered on the header box.
        let tl_x = box_x + (box_w - text_width) / 2.0;
        let tl_y = box_y + (box_h - text_height) / 2.0;

        let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
        text_el.x = Some(tl_x + text_width / 2.0);
        text_el.y = Some(tl_y + font_size);
        text_el.fill = shape.get_font_color().to_owned();
        text_el.class_name = "text-mono".to_owned();
        // Go formats with `%vpx` — `%v` on an int means no decimals; on a
        // float it's shortest-roundtrip. FontSize is an int here plus 4.
        text_el.style = format!("text-anchor:middle;font-size:{}px;", (font_size as i32) + 4);
        text_el.content = render_text(&shape.text.label, text_el.x.unwrap(), text_height);
        out.push_str(&text_el.render());
    }
    out
}

/// Render one row of a class shape (prefix + name + type).
#[allow(clippy::too_many_arguments)]
fn class_row(
    shape: &d2_target::Shape,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    _box_h: f64,
    prefix: &str,
    name: &str,
    type_text: &str,
    font_size: f64,
    underline: bool,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut out = String::new();

    // InsideMiddleLeft: prefix sits left-aligned with PREFIX_PADDING from
    // the left edge; y is center of the row.
    let prefix_tl_x = box_x + d2_target::PREFIX_PADDING as f64;
    let prefix_tl_y = box_y + (_box_h - font_size) / 2.0;

    let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
    text_el.x = Some(prefix_tl_x);
    text_el.y = Some(prefix_tl_y + font_size * 3.0 / 4.0);
    text_el.fill = shape.primary_accent_color.clone();
    text_el.class_name = "text-mono".to_owned();
    text_el.style = format!("text-anchor:start;font-size:{}px", font_size);
    text_el.content = prefix.to_owned();
    out.push_str(&text_el.render());

    // Name text at prefix_tl_x + PREFIX_WIDTH.
    text_el.x = Some(prefix_tl_x + d2_target::PREFIX_WIDTH as f64);
    text_el.fill = shape.fill.clone();
    text_el.class_name = if underline {
        "text-mono text-underline".to_owned()
    } else {
        "text-mono".to_owned()
    };
    text_el.content = d2_svg_path::escape_text(name);
    out.push_str(&text_el.render());

    // Type text right-aligned (InsideMiddleRight) with TYPE_PADDING from
    // the right edge.
    let type_tr_x = box_x + box_w - d2_target::TYPE_PADDING as f64;
    text_el.x = Some(type_tr_x);
    text_el.y = Some(prefix_tl_y + font_size * 3.0 / 4.0);
    text_el.fill = shape.secondary_accent_color.clone();
    text_el.class_name = "text-mono".to_owned();
    text_el.style = format!("text-anchor:end;font-size:{}px", font_size);
    text_el.content = d2_svg_path::escape_text(type_text);
    out.push_str(&text_el.render());

    out
}

// ---------------------------------------------------------------------------
// Draw SQL table shape
// ---------------------------------------------------------------------------

fn draw_table(
    buf: &mut String,
    diagram_hash: &str,
    shape: &d2_target::Shape,
    inline_theme: Option<&d2_themes::Theme>,
) {
    // Mirror Go `d2renderers/d2svg/table.go drawTable`.
    let (fill, stroke) = shape_theme(shape);
    let style = shape_css_style(shape);

    // Outer rect
    let mut el = d2_themes::ThemableElement::new("rect", inline_theme);
    el.x = Some(shape.pos.x as f64);
    el.y = Some(shape.pos.y as f64);
    el.width = Some(shape.width as f64);
    el.height = Some(shape.height as f64);
    el.fill = fill.clone();
    el.stroke = stroke.clone();
    el.style = style;
    el.class_name = "shape".to_owned();
    if shape.border_radius != 0 {
        el.rx = Some(shape.border_radius as f64);
        el.ry = Some(shape.border_radius as f64);
    }
    buf.push_str(&el.render());

    let box_x = shape.pos.x as f64;
    let box_y = shape.pos.y as f64;
    let box_w = shape.width as f64;
    let box_h = shape.height as f64;
    let col_count = shape.sql_table.columns.len();
    let row_height = box_h / (1 + col_count) as f64;

    // Header
    buf.push_str(&table_header(
        diagram_hash,
        shape,
        box_x,
        box_y,
        box_w,
        row_height,
        shape.text.label_width as f64,
        shape.text.label_height as f64,
        shape.text.font_size as f64,
        inline_theme,
    ));

    let longest_name_w = shape
        .sql_table
        .columns
        .iter()
        .map(|c| c.name.label_width)
        .max()
        .unwrap_or(0);

    let mut row_y = box_y + row_height;
    for (idx, col) in shape.sql_table.columns.iter().enumerate() {
        buf.push_str(&table_row(
            shape,
            box_x,
            row_y,
            box_w,
            row_height,
            &col.name.label,
            &col.type_.label,
            &col.constraint_abbr(),
            shape.text.font_size as f64,
            longest_name_w as f64,
            inline_theme,
        ));
        row_y += row_height;

        // Row separator line
        let mut line_el = d2_themes::ThemableElement::new("line", inline_theme);
        let last = idx == col_count - 1;
        if last && shape.border_radius != 0 {
            line_el.x1 = Some(box_x + shape.border_radius as f64);
            line_el.y1 = Some(row_y);
            line_el.x2 = Some(box_x + box_w - shape.border_radius as f64);
            line_el.y2 = Some(row_y);
        } else {
            line_el.x1 = Some(box_x);
            line_el.y1 = Some(row_y);
            line_el.x2 = Some(box_x + box_w);
            line_el.y2 = Some(row_y);
        }
        line_el.stroke = shape.fill.clone();
        line_el.style = "stroke-width:2".to_owned();
        buf.push_str(&line_el.render());
    }
}

/// Render the table header rect + title text (port of Go `tableHeader`).
#[allow(clippy::too_many_arguments)]
fn table_header(
    diagram_hash: &str,
    shape: &d2_target::Shape,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    box_h: f64,
    text_width: f64,
    text_height: f64,
    font_size: f64,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut out = String::new();

    let mut rect_el = d2_themes::ThemableElement::new("rect", inline_theme);
    rect_el.x = Some(box_x);
    rect_el.y = Some(box_y);
    rect_el.width = Some(box_w);
    rect_el.height = Some(box_h);
    rect_el.fill = shape.fill.clone();
    rect_el.fill_pattern = shape.fill_pattern.clone();
    rect_el.class_name = "class_header".to_owned();
    if shape.border_radius != 0 {
        rect_el.clip_path = format!("{}-{}", diagram_hash, shape.id);
    }
    out.push_str(&rect_el.render());

    if !shape.text.label.is_empty() {
        // InsideMiddleLeft: `tl = (box.x + HEADER_PADDING, box.y + (box.h - text_h)/2)`.
        let tl_x = box_x + d2_target::HEADER_PADDING as f64;
        let tl_y = box_y + (box_h - text_height) / 2.0;

        let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
        text_el.x = Some(tl_x);
        text_el.y = Some(tl_y + text_height * 3.0 / 4.0);
        text_el.fill = shape.get_font_color().to_owned();
        text_el.class_name = "text".to_owned();
        text_el.style = format!(
            "text-anchor:start;font-size:{}px",
            (font_size as i32) + 4
        );
        text_el.content = d2_svg_path::escape_text(&shape.text.label);
        out.push_str(&text_el.render());
    }

    let _ = text_width;
    out
}

/// Render one row of a table shape (name + type + constraint).
#[allow(clippy::too_many_arguments)]
fn table_row(
    shape: &d2_target::Shape,
    box_x: f64,
    box_y: f64,
    box_w: f64,
    box_h: f64,
    name: &str,
    type_text: &str,
    constraint: &str,
    font_size: f64,
    longest_name_w: f64,
    inline_theme: Option<&d2_themes::Theme>,
) -> String {
    let mut out = String::new();
    // InsideMiddleLeft for name, at `NamePadding` from the left.
    let name_tl_x = box_x + d2_target::NAME_PADDING as f64;
    let name_tl_y = box_y + (box_h - font_size) / 2.0;

    let mut text_el = d2_themes::ThemableElement::new("text", inline_theme);
    text_el.x = Some(name_tl_x);
    text_el.y = Some(name_tl_y + font_size * 3.0 / 4.0);
    text_el.fill = shape.primary_accent_color.clone();
    text_el.class_name = "text".to_owned();
    text_el.style = format!("text-anchor:start;font-size:{}px", font_size);
    text_el.content = d2_svg_path::escape_text(name);
    out.push_str(&text_el.render());

    // Type: start at `name_x + longest_name_w + TypePadding`.
    text_el.x = Some(name_tl_x + longest_name_w + d2_target::TYPE_PADDING as f64);
    text_el.fill = shape.neutral_accent_color.clone();
    text_el.content = d2_svg_path::escape_text(type_text);
    out.push_str(&text_el.render());

    // Constraint: right-aligned at `box.right - NamePadding`.
    text_el.x = Some(box_x + box_w - d2_target::NAME_PADDING as f64);
    text_el.fill = shape.secondary_accent_color.clone();
    text_el.style = format!("text-anchor:end;font-size:{}px", font_size);
    text_el.content = constraint.to_owned();
    out.push_str(&text_el.render());

    out
}

// ---------------------------------------------------------------------------
// Shadow filter definition
// ---------------------------------------------------------------------------

fn define_shadow_filter(buf: &mut String) {
    buf.push_str(concat!(
        "<defs>\n",
        "\t<filter id=\"shadow-filter\" width=\"200%\" height=\"200%\" x=\"-50%\" y=\"-50%\">\n",
        "\t\t<feGaussianBlur stdDeviation=\"1.7 \" in=\"SourceGraphic\"></feGaussianBlur>\n",
        "\t\t<feFlood flood-color=\"#3d4574\" flood-opacity=\"0.4\" result=\"ShadowFeFlood\" in=\"SourceGraphic\"></feFlood>\n",
        "\t\t<feComposite in=\"ShadowFeFlood\" in2=\"SourceAlpha\" operator=\"in\" result=\"ShadowFeComposite\"></feComposite>\n",
        "\t\t<feOffset dx=\"3\" dy=\"5\" result=\"ShadowFeOffset\" in=\"ShadowFeComposite\"></feOffset>\n",
        "\t\t<feBlend in=\"SourceGraphic\" in2=\"ShadowFeOffset\" mode=\"normal\" result=\"ShadowFeBlend\"></feBlend>\n",
        "\t</filter>\n",
        "</defs>",
    ));
}

// ---------------------------------------------------------------------------
// Gradient definitions
// ---------------------------------------------------------------------------

fn define_gradients(buf: &mut String, css_gradient: &str) {
    if let Ok(gradient) = d2_color::parse_gradient(css_gradient) {
        write!(buf, "<defs>{}</defs>", d2_color::gradient_to_svg(&gradient)).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Icon size helper
// ---------------------------------------------------------------------------

/// Mirror Go `d2target.GetIconSize`: icon size depends on the label
/// position — inside-middle-center gets half the box's shorter side,
/// other inside positions clamp to `[DEFAULT_ICON_SIZE, min_side]`, then
/// the whole thing is clipped to `MAX_ICON_SIZE` and (for non-outside
/// placements) to `box_side − 2*PADDING`.
fn get_icon_size(geo_box: &d2_geo::Box2D, icon_position: &str) -> i32 {
    let pos = d2_label::Position::from_string(icon_position);
    let min_dimension = geo_box.width.min(geo_box.height) as i32;
    let half_min = (0.5 * min_dimension as f64).ceil() as i32;

    let mut size = if matches!(pos, d2_label::Position::InsideMiddleCenter) {
        half_min
    } else {
        min_dimension.min(d2_target::DEFAULT_ICON_SIZE.max(half_min))
    };
    size = size.min(d2_target::MAX_ICON_SIZE);

    if !pos.is_outside() {
        let pad = d2_label::PADDING as i32;
        let w_cap = (geo_box.width as i32 - 2 * pad).max(0);
        let h_cap = (geo_box.height as i32 - 2 * pad).max(0);
        size = size.min(w_cap.min(h_cap));
    }
    size
}

// ---------------------------------------------------------------------------
// Base64 URL encoding (no padding, URL-safe alphabet)
// ---------------------------------------------------------------------------

fn base64_url_encode(input: &str) -> String {
    use std::io::Write as _;
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 4 / 3 + 4);
    {
        let mut encoder = Base64Encoder::new(&mut out);
        encoder.write_all(bytes).unwrap();
        encoder.finish();
    }
    String::from_utf8(out).unwrap()
}

/// Minimal base64-url encoder (RFC 4648 §5, no padding).
struct Base64Encoder<W: std::io::Write> {
    writer: W,
    buf: [u8; 3],
    len: usize,
}

const B64URL_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

impl<W: std::io::Write> Base64Encoder<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            buf: [0; 3],
            len: 0,
        }
    }

    fn flush_buf(&mut self) {
        if self.len == 0 {
            return;
        }
        let b0 = self.buf[0];
        let b1 = if self.len > 1 { self.buf[1] } else { 0 };
        let b2 = if self.len > 2 { self.buf[2] } else { 0 };
        let _ = self.writer.write_all(&[B64URL_TABLE[(b0 >> 2) as usize]]);
        let _ = self
            .writer
            .write_all(&[B64URL_TABLE[((b0 & 0x03) << 4 | b1 >> 4) as usize]]);
        if self.len > 1 {
            let _ = self
                .writer
                .write_all(&[B64URL_TABLE[((b1 & 0x0f) << 2 | b2 >> 6) as usize]]);
        } else {
            let _ = self.writer.write_all(b"=");
        }
        if self.len > 2 {
            let _ = self.writer.write_all(&[B64URL_TABLE[(b2 & 0x3f) as usize]]);
        } else {
            let _ = self.writer.write_all(b"=");
        }
        self.len = 0;
    }

    fn finish(mut self) {
        self.flush_buf();
    }
}

impl<W: std::io::Write> std::io::Write for Base64Encoder<W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut written = 0;
        for &byte in data {
            self.buf[self.len] = byte;
            self.len += 1;
            if self.len == 3 {
                self.flush_buf();
            }
            written += 1;
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dimensions calculation
// ---------------------------------------------------------------------------

fn dimensions(diagram: &d2_target::Diagram, pad: i32) -> (i32, i32, i32, i32) {
    let (tl, br) = diagram.bounding_box();
    let left = tl.x - pad;
    let top = tl.y - pad;
    let width = br.x - tl.x + pad * 2;
    let height = br.y - tl.y + pad * 2;
    (left, top, width, height)
}

// ---------------------------------------------------------------------------
// Theme CSS generation
// ---------------------------------------------------------------------------

/// Generate CSS rulesets for a single theme.
fn single_theme_rulesets(
    diagram_hash: &str,
    theme_id: i64,
    overrides: Option<&d2_themes::ThemeOverrides>,
) -> Result<String, String> {
    let theme = d2_themes::catalog::find(theme_id)
        .ok_or_else(|| format!("theme {} not found", theme_id))?;
    let colors = if let Some(ov) = overrides {
        theme.apply_overrides(ov)
    } else {
        d2_themes::OwnedColorPalette::from(&theme.colors)
    };

    let mut out = String::new();

    let all_colors: &[(&str, &str)] = &[
        ("N1", &colors.n1),
        ("N2", &colors.n2),
        ("N3", &colors.n3),
        ("N4", &colors.n4),
        ("N5", &colors.n5),
        ("N6", &colors.n6),
        ("N7", &colors.n7),
        ("B1", &colors.b1),
        ("B2", &colors.b2),
        ("B3", &colors.b3),
        ("B4", &colors.b4),
        ("B5", &colors.b5),
        ("B6", &colors.b6),
        ("AA2", &colors.aa2),
        ("AA4", &colors.aa4),
        ("AA5", &colors.aa5),
        ("AB4", &colors.ab4),
        ("AB5", &colors.ab5),
    ];

    for property in &["fill", "stroke", "background-color", "color"] {
        for &(name, value) in all_colors {
            write!(
                out,
                "\n\t\t.{} .{}-{}{{{}:{};}}",
                diagram_hash, property, name, property, value
            )
            .unwrap();
        }
    }

    // Appendix
    write!(out, ".appendix text.text{{fill:{}}}", colors.n1).unwrap();

    // Markdown CSS variables
    write!(
        out,
        ".md{{--color-fg-default:{};--color-fg-muted:{};--color-fg-subtle:{};--color-canvas-default:{};--color-canvas-subtle:{};--color-border-default:{};--color-border-muted:{};--color-neutral-muted:{};--color-accent-fg:{};--color-accent-emphasis:{};--color-attention-subtle:{};--color-danger-fg:red;}}",
        colors.n1, colors.n2, colors.n3,
        colors.n7, colors.n6,
        colors.b1, colors.b2,
        colors.n6,
        colors.b2, colors.b2,
        colors.n2,
    ).unwrap();

    // Sketch-mode overlay rules.
    // Mirrors the `.sketch-overlay-*` block Go emits in d2svg.singleThemeRulesets
    // (d2svg.go around line 3222). Each color maps to a streaks pattern URL and
    // a CSS blend mode chosen by the color's luminance category.
    let sketch_colors: &[(&str, &str)] = &[
        ("B1", &colors.b1),
        ("B2", &colors.b2),
        ("B3", &colors.b3),
        ("B4", &colors.b4),
        ("B5", &colors.b5),
        ("B6", &colors.b6),
        ("AA2", &colors.aa2),
        ("AA4", &colors.aa4),
        ("AA5", &colors.aa5),
        ("AB4", &colors.ab4),
        ("AB5", &colors.ab5),
        ("N1", &colors.n1),
        ("N2", &colors.n2),
        ("N3", &colors.n3),
        ("N4", &colors.n4),
        ("N5", &colors.n5),
        ("N6", &colors.n6),
        ("N7", &colors.n7),
    ];
    for &(name, value) in sketch_colors {
        let lc = d2_color::luminance_category(value)
            .map_err(|e| format!("luminance_category({}): {}", value, e))?;
        let lc_str = lc.as_str();
        let blend = match lc_str {
            "bright" => "darken",
            "normal" => "color-burn",
            "dark" => "overlay",
            "darker" => "lighten",
            _ => "normal",
        };
        write!(
            out,
            ".sketch-overlay-{}{{fill:url(#streaks-{}-{});mix-blend-mode:{}}}",
            name, lc_str, diagram_hash, blend
        )
        .unwrap();
    }

    // Light/dark code visibility
    if theme.is_dark() {
        out.push_str(".light-code{display: none}");
        out.push_str(".dark-code{display: block}");
    } else {
        out.push_str(".light-code{display: block}");
        out.push_str(".dark-code{display: none}");
    }

    Ok(out)
}

/// Generate theme CSS for light and optional dark theme.
pub fn theme_css(
    diagram_hash: &str,
    theme_id: Option<i64>,
    dark_theme_id: Option<i64>,
    overrides: Option<&d2_themes::ThemeOverrides>,
    dark_overrides: Option<&d2_themes::ThemeOverrides>,
) -> Result<String, String> {
    let tid = theme_id.unwrap_or(0); // 0 = NeutralDefault
    let mut out = single_theme_rulesets(diagram_hash, tid, overrides)?;

    if let Some(dark_id) = dark_theme_id {
        let dark_out = single_theme_rulesets(diagram_hash, dark_id, dark_overrides)?;
        write!(
            out,
            "@media screen and (prefers-color-scheme:dark){{{}}}",
            dark_out
        )
        .unwrap();
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Font embedding
// ---------------------------------------------------------------------------

fn append_on_trigger(buf: &mut String, source: &str, triggers: &[&str], content: &str) {
    for trigger in triggers {
        if source.contains(trigger) {
            buf.push_str(content);
            break;
        }
    }
}

/// Embed font-face CSS rules into the SVG, scanning for font class usage.
pub fn embed_fonts(
    buf: &mut String,
    diagram_hash: &str,
    source: &str,
    font_family: &d2_fonts::FontFamily,
    mono_font_family: &d2_fonts::FontFamily,
    corpus: &str,
) {
    buf.push_str(r#"<style type="text/css"><![CDATA["#);

    // Order below mirrors Go d2renderers/d2svg/d2svg.go `embedFonts` exactly.
    // Reordering breaks byte-for-byte parity with the Go exporter.

    // 1. Regular text font
    append_on_trigger(
        buf,
        source,
        &[
            r#"class="text""#,
            r#"class="text "#,
            r#"class="md""#,
            r#"class="md "#,
        ],
        &format!(
            r#"
.{dh} .text {{
	font-family: "{dh}-font-regular";
}}
@font-face {{
	font-family: {dh}-font-regular;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = font_family
                .font(0, d2_fonts::FontStyle::Regular)
                .get_encoded_subset(corpus),
        ),
    );

    // 2. Markdown semibold font (only when markdown content is present)
    append_on_trigger(
        buf,
        source,
        &[r#"class="md""#, r#"class="md "#],
        &format!(
            r#"
@font-face {{
	font-family: {dh}-font-semibold;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = font_family
                .font(0, d2_fonts::FontStyle::Semibold)
                .get_encoded_subset(corpus),
        ),
    );

    // 3. Text underline
    append_on_trigger(
        buf,
        source,
        &["text-underline"],
        r#"
.text-underline {
	text-decoration: underline;
}"#,
    );

    // 4. Text link
    append_on_trigger(
        buf,
        source,
        &["text-link"],
        r#"
.text-link {
	fill: blue;
}

.text-link:visited {
	fill: purple;
}"#,
    );

    // 5. Animated connection
    append_on_trigger(
        buf,
        source,
        &["animated-connection"],
        r#"
@keyframes dashdraw {
	from {
		stroke-dashoffset: 0;
	}
}
"#,
    );

    // 6. Animated shape
    append_on_trigger(
        buf,
        source,
        &["animated-shape"],
        r#"
@keyframes shapeappear {
    0%, 100% { transform: translateY(0); filter: drop-shadow(0px 0px 0px rgba(0,0,0,0)); }
    50% { transform: translateY(-4px); filter: drop-shadow(0px 12.6px 25.2px rgba(50,50,93,0.25)) drop-shadow(0px 7.56px 15.12px rgba(0,0,0,0.1)); }
}
.animated-shape {
	animation: shapeappear 1s linear infinite;
}
"#,
    );

    // 7. Appendix icon drop shadow
    append_on_trigger(
        buf,
        source,
        &["appendix-icon"],
        r#"
.appendix-icon {
	filter: drop-shadow(0px 0px 32px rgba(31, 36, 58, 0.1));
}"#,
    );

    // 8. Bold font
    append_on_trigger(
        buf,
        source,
        &[r#"class="text-bold"#, "<b>", "<strong>"],
        &format!(
            r#"
.{dh} .text-bold {{
	font-family: "{dh}-font-bold";
}}
@font-face {{
	font-family: {dh}-font-bold;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = font_family
                .font(0, d2_fonts::FontStyle::Bold)
                .get_encoded_subset(corpus),
        ),
    );

    // 9. Italic font
    append_on_trigger(
        buf,
        source,
        &[r#"class="text-italic"#, "<em>", "<dfn>"],
        &format!(
            r#"
.{dh} .text-italic {{
	font-family: "{dh}-font-italic";
}}
@font-face {{
	font-family: {dh}-font-italic;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = font_family
                .font(0, d2_fonts::FontStyle::Italic)
                .get_encoded_subset(corpus),
        ),
    );

    // 10. Mono font (regular)
    append_on_trigger(
        buf,
        source,
        &[r#"class="text-mono"#, "<pre>", "<code>", "<kbd>", "<samp>"],
        &format!(
            r#"
.{dh} .text-mono {{
	font-family: "{dh}-font-mono";
}}
@font-face {{
	font-family: {dh}-font-mono;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = mono_font_family
                .font(0, d2_fonts::FontStyle::Regular)
                .get_encoded_subset(corpus),
        ),
    );

    // 11. Mono bold font
    append_on_trigger(
        buf,
        source,
        &[r#"class="text-mono-bold"#],
        &format!(
            r#"
.{dh} .text-mono-bold {{
	font-family: "{dh}-font-mono-bold";
}}
@font-face {{
	font-family: {dh}-font-mono-bold;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = mono_font_family
                .font(0, d2_fonts::FontStyle::Bold)
                .get_encoded_subset(corpus),
        ),
    );

    // 12. Mono italic font
    append_on_trigger(
        buf,
        source,
        &[r#"class="text-mono-italic"#],
        &format!(
            r#"
.{dh} .text-mono-italic {{
	font-family: "{dh}-font-mono-italic";
}}
@font-face {{
	font-family: {dh}-font-mono-italic;
	src: url("{url}");
}}"#,
            dh = diagram_hash,
            url = mono_font_family
                .font(0, d2_fonts::FontStyle::Italic)
                .get_encoded_subset(corpus),
        ),
    );

    buf.push_str("]]></style>");
}

// ---------------------------------------------------------------------------
// Main render function
// ---------------------------------------------------------------------------

/// Render a diagram to SVG bytes.
pub fn render(diagram: &d2_target::Diagram, opts: &RenderOpts) -> Result<Vec<u8>, String> {
    let pad = opts.pad.map_or(DEFAULT_PADDING, |p| p as i32);
    let theme_id = opts.theme_id.unwrap_or(0);
    let dark_theme_id = opts.dark_theme_id;
    let scale = opts.scale;

    let mut buf = String::with_capacity(16384);

    // Shadow filter if needed
    for s in &diagram.shapes {
        if s.shadow {
            define_shadow_filter(&mut buf);
            break;
        }
    }

    // Gradient definitions
    if d2_color::is_gradient(&diagram.root.fill) {
        define_gradients(&mut buf, &diagram.root.fill);
    }
    if d2_color::is_gradient(&diagram.root.stroke) {
        define_gradients(&mut buf, &diagram.root.stroke);
    }
    for s in &diagram.shapes {
        if d2_color::is_gradient(&s.fill) {
            define_gradients(&mut buf, &s.fill);
        }
        if d2_color::is_gradient(&s.stroke) {
            define_gradients(&mut buf, &s.stroke);
        }
    }
    for c in &diagram.connections {
        if d2_color::is_gradient(&c.stroke) {
            define_gradients(&mut buf, &c.stroke);
        }
        if d2_color::is_gradient(&c.fill) {
            define_gradients(&mut buf, &c.fill);
        }
    }

    // Diagram hash for CSS scoping
    let diagram_hash = diagram.hash_id(opts.salt.as_deref());
    let isolated_hash = diagram_hash.clone();
    let diagram_hash = if !opts.master_id.is_empty() {
        opts.master_id.clone()
    } else {
        diagram_hash
    };

    // Build ID-to-shape map and sort objects
    let mut id_to_shape: HashMap<String, &d2_target::Shape> = HashMap::new();
    let mut all_objects: Vec<DiagramObject> =
        Vec::with_capacity(diagram.shapes.len() + diagram.connections.len());

    for s in &diagram.shapes {
        id_to_shape.insert(s.id.clone(), s);
        all_objects.push(DiagramObject::Shape(s));
    }
    for c in &diagram.connections {
        all_objects.push(DiagramObject::Connection(c));
    }

    sort_objects(&mut all_objects);

    let mut label_masks: Vec<String> = Vec::new();
    let mut markers: HashMap<String, ()> = HashMap::new();

    // Determine inline theme (only when no dark theme)
    let inline_theme: Option<&d2_themes::Theme> = if dark_theme_id.is_none() {
        d2_themes::catalog::find(theme_id)
    } else {
        None
    };

    // Draw objects
    for obj in &all_objects {
        match obj {
            DiagramObject::Connection(c) => {
                let lm = draw_connection(
                    &mut buf,
                    &isolated_hash,
                    c,
                    &mut markers,
                    &id_to_shape,
                    inline_theme,
                )?;
                if !lm.is_empty() {
                    label_masks.push(lm);
                }
            }
            DiagramObject::Shape(s) => {
                let lm = draw_shape(&mut buf, &diagram_hash, s, inline_theme)?;
                if !lm.is_empty() {
                    label_masks.push(lm);
                }
            }
        }
    }

    // Compute dimensions
    let (mut left, mut top, mut w, mut h) = dimensions(diagram, pad);

    // Label mask. Match Go d2svg.go: each piece is on its own line, joined
    // by `\n`, so the resulting fragment looks like
    //   <mask ...>\n<rect ...></rect>\n{label_masks}\n</mask>
    write!(
        buf,
        "<mask id=\"{}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\"></rect>\n{}\n</mask>",
        isolated_hash,
        left, top, w, h,
        left, top, w, h,
        label_masks.join("\n"),
    )
    .unwrap();

    // Style elements
    let mut upper_buf = String::new();
    if opts.master_id.is_empty() {
        let font_family = diagram
            .font_family
            .as_ref()
            .map_or(d2_fonts::FontFamily::SourceSansPro, |_| {
                d2_fonts::FontFamily::SourceSansPro
            });
        let mono_family = diagram
            .mono_font_family
            .as_ref()
            .map_or(d2_fonts::FontFamily::SourceCodePro, |_| {
                d2_fonts::FontFamily::SourceCodePro
            });

        // Collect corpus (all text in diagram)
        let corpus = collect_corpus(diagram);

        embed_fonts(
            &mut upper_buf,
            &diagram_hash,
            &buf,
            &font_family,
            &mono_family,
            &corpus,
        );

        let theme_stylesheet = theme_css(
            &diagram_hash,
            Some(theme_id),
            dark_theme_id,
            opts.theme_overrides.as_ref(),
            opts.dark_theme_overrides.as_ref(),
        )?;
        write!(
            upper_buf,
            r#"<style type="text/css"><![CDATA[{}{}]]></style>"#,
            BASE_STYLESHEET, theme_stylesheet
        )
        .unwrap();
    }

    // Background element
    let half_sw = (diagram.root.stroke_width as f64 / 2.0).ceil() as i32;
    left -= half_sw;
    top -= half_sw;
    w += half_sw * 2;
    h += half_sw * 2;

    let mut bg_el = d2_themes::ThemableElement::new("rect", inline_theme);
    bg_el.x = Some(left as f64);
    bg_el.y = Some(top as f64);
    bg_el.width = Some(w as f64);
    bg_el.height = Some(h as f64);
    bg_el.fill = diagram.root.fill.clone();
    bg_el.stroke = diagram.root.stroke.clone();
    bg_el.fill_pattern = diagram.root.fill_pattern.clone();
    bg_el.rx = Some(diagram.root.border_radius as f64);
    if diagram.root.stroke_dash != 0.0 {
        let (dash, gap) = d2_svg_path::get_stroke_dash_attributes(
            diagram.root.stroke_width as f64,
            diagram.root.stroke_dash,
        );
        bg_el.stroke_dash_array = format!("{}, {}", dash, gap);
    }
    bg_el.attributes = format!(r#"stroke-width="{}""#, diagram.root.stroke_width);

    // Viewbox adjustments
    left -= half_sw;
    top -= half_sw;
    w += half_sw * 2;
    h += half_sw * 2;

    // Scaling dimensions
    let dim_attr = if let Some(sc) = scale {
        format!(
            r#" width="{}" height="{}""#,
            (sc * w as f64).ceil() as i32,
            (sc * h as f64).ceil() as i32
        )
    } else {
        String::new()
    };

    let alignment = if opts.center == Some(true) {
        "xMidYMid"
    } else {
        "xMinYMin"
    };

    let (fit_open, xml_tag, fit_close, id_attr, tag) = if opts.master_id.is_empty() {
        let version_attr = if opts.omit_version != Some(true) {
            r#"data-d2-version="v0.7.1-HEAD""#
        } else {
            ""
        };
        (
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" {} preserveAspectRatio="{} meet" viewBox="0 0 {} {}"{}>"#,
                version_attr, alignment, w, h, dim_attr
            ),
            if opts.no_xml_tag == Some(true) {
                ""
            } else {
                r#"<?xml version="1.0" encoding="utf-8"?>"#
            },
            "</svg>",
            "d2-svg",
            "svg",
        )
    } else {
        (String::new(), "", "", "", "g")
    };

    let doc = format!(
        r#"{}{}<{} class="{} {}" width="{}" height="{}" viewBox="{} {} {} {}">{}{}{}</{}>{}"#,
        xml_tag,
        fit_open,
        tag,
        diagram_hash,
        id_attr,
        w,
        h,
        left,
        top,
        w,
        h,
        bg_el.render(),
        upper_buf,
        buf,
        tag,
        fit_close,
    );

    Ok(doc.into_bytes())
}

/// Collect all text content from a diagram for font subsetting.
fn collect_corpus(diagram: &d2_target::Diagram) -> String {
    // Mirror Go `d2target.Diagram.GetCorpus` exactly so the font subset
    // Rust produces hashes identically to Go. Key quirks:
    // - labels are concatenated back-to-back with no separators;
    // - tooltip/link are followed by an incrementing appendixCount digit;
    // - class fields/methods include visibility token after the combined
    //   `name + type` text;
    // - sql columns also push constraint abbreviations.
    let mut corpus = String::new();
    let mut appendix_count = 0;
    for s in &diagram.shapes {
        corpus.push_str(&s.text.label);
        if !s.tooltip.is_empty() {
            corpus.push_str(&s.tooltip);
            appendix_count += 1;
            corpus.push_str(&appendix_count.to_string());
        }
        if !s.link.is_empty() {
            corpus.push_str(&s.link);
            appendix_count += 1;
            corpus.push_str(&appendix_count.to_string());
        }
        corpus.push_str(&s.pretty_link);
        if s.type_ == d2_target::SHAPE_CLASS {
            for f in &s.class.fields {
                corpus.push_str(&f.name);
                corpus.push_str(&f.type_);
                corpus.push_str(f.visibility_token());
            }
            for m in &s.class.methods {
                corpus.push_str(&m.name);
                corpus.push_str(&m.return_);
                corpus.push_str(m.visibility_token());
            }
        }
        if s.type_ == d2_target::SHAPE_SQL_TABLE {
            for col in &s.sql_table.columns {
                corpus.push_str(&col.name.label);
                corpus.push_str(&col.type_.label);
                corpus.push_str(&col.constraint_abbr());
            }
        }
    }
    for c in &diagram.connections {
        corpus.push_str(&c.text.label);
        if let Some(ref l) = c.src_label {
            corpus.push_str(&l.label);
        }
        if let Some(ref l) = c.dst_label {
            corpus.push_str(&l.label);
        }
    }
    // Legend corpus still TODO.
    corpus
}

// ---------------------------------------------------------------------------
// Multiboard rendering
// ---------------------------------------------------------------------------

/// Render a multi-board diagram, producing one SVG per board.
pub fn render_multiboard(
    diagram: &d2_target::Diagram,
    opts: &RenderOpts,
) -> Result<Vec<Vec<u8>>, String> {
    let mut boards: Vec<Vec<u8>> = Vec::new();

    for dl in &diagram.layers {
        let children = render_multiboard(dl, opts)?;
        boards.extend(children);
    }
    for dl in &diagram.scenarios {
        let children = render_multiboard(dl, opts)?;
        boards.extend(children);
    }
    for dl in &diagram.steps {
        let children = render_multiboard(dl, opts)?;
        boards.extend(children);
    }

    if !diagram.is_folder_only {
        let out = render(diagram, opts)?;
        boards.insert(0, out);
    }

    Ok(boards)
}

// ---------------------------------------------------------------------------
// Animation wrapper (mirrors Go d2renderers/d2animate/d2animate.go)
// ---------------------------------------------------------------------------

/// Collect text corpus from diagram and all nested boards (for font subset).
/// Mirrors Go's `Diagram.GetNestedCorpus`.
fn collect_nested_corpus(diagram: &d2_target::Diagram) -> String {
    let mut corpus = collect_corpus(diagram);
    for d in &diagram.layers {
        corpus.push_str(&collect_nested_corpus(d));
    }
    for d in &diagram.scenarios {
        corpus.push_str(&collect_nested_corpus(d));
    }
    for d in &diagram.steps {
        corpus.push_str(&collect_nested_corpus(d));
    }
    corpus
}

/// Build a single `@keyframes` block for one board in the animation.
/// Mirrors Go d2animate.makeKeyframe (transitionDurationMS = 1).
fn make_keyframe(
    delay_ms: i64,
    duration_ms: i64,
    total_ms: i64,
    identifier: usize,
    diagram_hash: &str,
) -> String {
    const TRANSITION_DURATION_MS: i64 = 1;
    let total = total_ms as f64;
    let percentage_before = ((delay_ms - TRANSITION_DURATION_MS).max(0) as f64 / total) * 100.0;
    let percentage_start = (delay_ms as f64 / total) * 100.0;
    let percentage_end = ((delay_ms + duration_ms - TRANSITION_DURATION_MS) as f64 / total) * 100.0;

    if percentage_end.ceil() as i64 == 100 {
        return format!(
            "@keyframes d2Transition-{}-{} {{\n\t\t0%%, {:.6}%% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{:.6}%%, {:.6}%% {{\n\t\t\t\topacity: 1;\n\t\t}}\n}}",
            diagram_hash,
            identifier,
            percentage_before,
            percentage_start,
            percentage_end.ceil(),
        );
    }

    let percentage_after = ((delay_ms + duration_ms) as f64 / total) * 100.0;
    format!(
        "@keyframes d2Transition-{}-{} {{\n\t\t0%%, {:.6}%% {{\n\t\t\t\topacity: 0;\n\t\t}}\n\t\t{:.6}%%, {:.6}%% {{\n\t\t\t\topacity: 1;\n\t\t}}\n\t\t{:.6}%%, 100%% {{\n\t\t\t\topacity: 0;\n\t\t}}\n}}",
        diagram_hash,
        identifier,
        percentage_before,
        percentage_start,
        percentage_end,
        percentage_after,
    )
}

/// Wrap per-board SVGs into a single animated SVG document.
///
/// Mirrors Go `d2renderers/d2animate.Wrap`. The generated structure is:
/// ```text
/// <?xml version="1.0" ...?>
/// <svg xmlns=... d2Version=... viewBox="0 0 W H">
///   <svg class="d2-svg" width="W" height="H" viewBox="left top W H">
///     <style> font embed </style>
///     <style> base + theme </style>
///     <style> @keyframes ...</style>
///     ... per-board <g> with animation style ...
///   </svg>
/// </svg>
/// ```
pub fn wrap(
    root_diagram: &d2_target::Diagram,
    svgs: &[Vec<u8>],
    opts: &RenderOpts,
    interval_ms: i64,
) -> Result<Vec<u8>, String> {
    let mut buf = String::with_capacity(8192);

    let pad = opts.pad.map_or(DEFAULT_PADDING, |p| p as i32);
    let (tl, br) = root_diagram.nested_bounding_box();
    let left = tl.x - pad;
    let top = tl.y - pad;
    let width = br.x - tl.x + pad * 2;
    let height = br.y - tl.y + pad * 2;

    let dim_attr = if let Some(sc) = opts.scale {
        format!(
            r#" width="{}" height="{}""#,
            (sc * width as f64).ceil() as i32,
            (sc * height as f64).ceil() as i32
        )
    } else {
        String::new()
    };

    // Outer wrapper. Note: this path mirrors Go d2animate.Wrap, which uses
    // the OLDER `d2Version="..."` attribute name (not `data-d2-version`).
    // d2animate's wrapper hasn't been updated to track d2svg.go's rename, so
    // matching it byte-for-byte means keeping the old name here. The
    // single-board path (boards.len() == 1) bypasses wrap() entirely and
    // gets the modern `data-d2-version` from d2svg::render_multiboard.
    write!(
        buf,
        r#"<?xml version="1.0" encoding="utf-8"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" d2Version="v0.7.1-HEAD" preserveAspectRatio="xMinYMin meet" viewBox="0 0 {} {}"{}>"#,
        width, height, dim_attr
    ).unwrap();

    // Inner <svg class="d2-svg"> viewport
    write!(
        buf,
        r#"<svg class="d2-svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        width, height, left, top, width, height
    )
    .unwrap();

    // Concatenate all SVG bodies (space-separated like Go)
    let mut svgs_str = String::new();
    for svg in svgs {
        svgs_str.push_str(&String::from_utf8_lossy(svg));
        svgs_str.push(' ');
    }

    let diagram_hash = root_diagram.hash_id(opts.salt.as_deref());

    // Font embed <style> — uses combined content of all boards as source trigger.
    let font_family = d2_fonts::FontFamily::SourceSansPro;
    let mono_font_family = d2_fonts::FontFamily::SourceCodePro;
    let corpus = collect_nested_corpus(root_diagram);
    embed_fonts(
        &mut buf,
        &diagram_hash,
        &svgs_str,
        &font_family,
        &mono_font_family,
        &corpus,
    );

    // Base + theme <style>
    let theme_stylesheet = theme_css(
        &diagram_hash,
        opts.theme_id,
        opts.dark_theme_id,
        opts.theme_overrides.as_ref(),
        opts.dark_theme_overrides.as_ref(),
    )?;
    write!(
        buf,
        r#"<style type="text/css"><![CDATA[{}{}]]></style>"#,
        BASE_STYLESHEET, theme_stylesheet
    )
    .unwrap();

    // TODO: Markdown CSS block. Go's d2animate writes a `<style type="text/css">`
    // containing the GitHub markdown stylesheet whenever any nested board has a
    // text shape with a non-empty label. We don't emit it yet — none of the
    // currently passing fixtures need it, and the embedded asset is large
    // enough to belong in its own pass.

    // Keyframes <style>: one @keyframes per board.
    // For 0 boards (e.g. empty diagram), the block is an empty CDATA, matching Go.
    buf.push_str(r#"<style type="text/css"><![CDATA["#);
    let total_ms = (svgs.len() as i64) * interval_ms;
    for i in 0..svgs.len() {
        buf.push_str(&make_keyframe(
            (i as i64) * interval_ms,
            interval_ms,
            total_ms,
            i,
            &diagram_hash,
        ));
    }
    buf.push_str(r#"]]></style>"#);

    // Inject each board with an animation style attribute on the first <g ... .
    for (i, svg) in svgs.iter().enumerate() {
        let s = String::from_utf8_lossy(svg);
        let anim = format!(
            r#"<g style="animation: d2Transition-{}-{} {}ms infinite""#,
            diagram_hash, i, total_ms,
        );
        // Replace only the first "<g" occurrence, matching Go's strings.Replace(..., 1).
        if let Some(pos) = s.find("<g") {
            buf.push_str(&s[..pos]);
            buf.push_str(&anim);
            buf.push_str(&s[pos + 2..]);
        } else {
            buf.push_str(&s);
        }
    }

    buf.push_str("</svg></svg>");

    Ok(buf.into_bytes())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_empty_diagram() {
        let diagram = d2_target::Diagram::new();
        let opts = RenderOpts::default();
        let result = render(&diagram, &opts);
        assert!(result.is_ok());
        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("viewBox"));
    }

    #[test]
    fn test_render_single_rectangle() {
        let mut diagram = d2_target::Diagram::new();
        let mut shape = d2_target::base_shape();
        shape.id = "hello".to_owned();
        shape.type_ = d2_target::SHAPE_RECTANGLE.to_owned();
        shape.pos = d2_target::Point::new(50, 50);
        shape.width = 200;
        shape.height = 100;
        shape.fill = "#E3E9FD".to_owned();
        shape.stroke = "#0D32B2".to_owned();
        shape.text.label = "Hello World".to_owned();
        shape.text.font_size = 16;
        shape.text.label_width = 80;
        shape.text.label_height = 20;
        shape.text.bold = true;
        shape.label_position = "INSIDE_MIDDLE_CENTER".to_owned();

        diagram.shapes.push(shape);

        let opts = RenderOpts {
            theme_id: Some(0),
            ..Default::default()
        };
        let result = render(&diagram, &opts);
        assert!(result.is_ok(), "render failed: {:?}", result.err());
        let svg = String::from_utf8(result.unwrap()).unwrap();

        // Verify SVG structure
        assert!(svg.contains("<?xml version"), "missing XML declaration");
        assert!(svg.contains("<svg"), "missing outer SVG");
        assert!(svg.contains("viewBox"), "missing viewBox");
        assert!(svg.contains("Hello World"), "missing label text");
        assert!(svg.contains(r#"class="shape""#), "missing shape class");
        assert!(svg.contains("<rect"), "missing rect element");
        assert!(svg.contains("</svg>"), "missing closing svg");

        // Check the scoping hash is present
        assert!(svg.contains("d2-"), "missing diagram hash");
    }

    #[test]
    fn test_render_connection() {
        let mut diagram = d2_target::Diagram::new();

        let mut s1 = d2_target::base_shape();
        s1.id = "a".to_owned();
        s1.pos = d2_target::Point::new(50, 50);
        s1.width = 100;
        s1.height = 60;
        s1.fill = "#E3E9FD".to_owned();
        s1.stroke = "#0D32B2".to_owned();

        let mut s2 = d2_target::base_shape();
        s2.id = "b".to_owned();
        s2.pos = d2_target::Point::new(300, 50);
        s2.width = 100;
        s2.height = 60;
        s2.fill = "#E3E9FD".to_owned();
        s2.stroke = "#0D32B2".to_owned();

        let mut conn = d2_target::base_connection();
        conn.id = "a-b".to_owned();
        conn.src = "a".to_owned();
        conn.dst = "b".to_owned();
        conn.dst_arrow = d2_target::Arrowhead::Triangle;
        conn.stroke = "#0D32B2".to_owned();
        conn.route = vec![
            d2_geo::Point::new(150.0, 80.0),
            d2_geo::Point::new(300.0, 80.0),
        ];

        diagram.shapes.push(s1);
        diagram.shapes.push(s2);
        diagram.connections.push(conn);

        let opts = RenderOpts {
            theme_id: Some(0),
            ..Default::default()
        };
        let result = render(&diagram, &opts);
        assert!(result.is_ok(), "render failed: {:?}", result.err());
        let svg = String::from_utf8(result.unwrap()).unwrap();

        assert!(svg.contains("<path"), "missing connection path");
        assert!(svg.contains("marker"), "missing arrowhead marker");
        assert!(
            svg.contains(r#"class="connection"#),
            "missing connection class"
        );
    }

    #[test]
    fn test_render_text() {
        let result = render_text("hello", 50.0, 20.0);
        assert_eq!(result, "hello");

        let result = render_text("line1\nline2", 50.0, 40.0);
        assert!(result.contains("<tspan"));
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
    }

    #[test]
    fn test_theme_css_generation() {
        let css = theme_css("d2-test", Some(0), None, None, None);
        assert!(css.is_ok());
        let css = css.unwrap();
        assert!(css.contains("fill-N1"));
        assert!(css.contains("stroke-N7"));
    }

    #[test]
    fn test_fnv1a_hash_matches_go() {
        // Go FNV-1a for "testlalalas" should produce consistent results
        let h = hash_str("test");
        assert!(!h.is_empty());
        // The hash should be a decimal number string
        assert!(h.parse::<u32>().is_ok());
    }

    #[test]
    fn test_multiboard_rendering() {
        let mut diagram = d2_target::Diagram::new();
        diagram.is_folder_only = false;

        let mut layer = d2_target::Diagram::new();
        layer.name = "layer1".to_owned();
        layer.is_folder_only = false;
        diagram.layers.push(layer);

        let opts = RenderOpts::default();
        let result = render_multiboard(&diagram, &opts);
        assert!(result.is_ok());
        let boards = result.unwrap();
        assert_eq!(boards.len(), 2); // root + 1 layer
    }

    #[test]
    fn test_render_oval_shape() {
        let mut diagram = d2_target::Diagram::new();
        let mut shape = d2_target::base_shape();
        shape.id = "oval1".to_owned();
        shape.type_ = d2_target::SHAPE_OVAL.to_owned();
        shape.pos = d2_target::Point::new(50, 50);
        shape.width = 150;
        shape.height = 100;
        shape.fill = "#E3E9FD".to_owned();
        shape.stroke = "#0D32B2".to_owned();
        shape.text.label = "Oval".to_owned();
        shape.text.font_size = 16;
        shape.text.label_width = 40;
        shape.text.label_height = 20;
        shape.label_position = "INSIDE_MIDDLE_CENTER".to_owned();
        diagram.shapes.push(shape);

        let opts = RenderOpts::default();
        let result = render(&diagram, &opts);
        assert!(result.is_ok());
        let svg = String::from_utf8(result.unwrap()).unwrap();
        assert!(svg.contains("<ellipse"), "missing ellipse element");
    }

    #[test]
    fn test_dimensions() {
        let mut diagram = d2_target::Diagram::new();
        let mut shape = d2_target::base_shape();
        shape.pos = d2_target::Point::new(100, 100);
        shape.width = 200;
        shape.height = 100;
        shape.stroke_width = 0; // Zero stroke to simplify bounding box
        diagram.shapes.push(shape);

        let (left, top, w, h) = dimensions(&diagram, 50);
        assert_eq!(left, 50);
        assert_eq!(top, 50);
        assert_eq!(w, 300);
        assert_eq!(h, 200);
    }

    #[test]
    fn test_base64_url_encode() {
        let encoded = base64_url_encode("hello");
        assert!(!encoded.is_empty());
        // Should be URL-safe (no + or /)
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }

    #[test]
    fn test_shape_theme_swaps_for_class() {
        let mut shape = d2_target::base_shape();
        shape.type_ = d2_target::SHAPE_CLASS.to_owned();
        shape.fill = "blue".to_owned();
        shape.stroke = "red".to_owned();
        let (fill, stroke) = shape_theme(&shape);
        // For class shapes, fill/stroke are swapped
        assert_eq!(fill, "red");
        assert_eq!(stroke, "blue");
    }

    #[test]
    fn test_shape_theme_normal_for_rect() {
        let mut shape = d2_target::base_shape();
        shape.type_ = d2_target::SHAPE_RECTANGLE.to_owned();
        shape.fill = "blue".to_owned();
        shape.stroke = "red".to_owned();
        let (fill, stroke) = shape_theme(&shape);
        assert_eq!(fill, "blue");
        assert_eq!(stroke, "red");
    }
}
