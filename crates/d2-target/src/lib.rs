//! d2-target: diagram, shape, and connection types for d2 rendering.
//!
//! Ported from Go `d2target/d2target.go`, `d2target/class.go`,
//! and `d2target/sqltable.go`.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use d2_color;
use d2_themes;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DEFAULT_ICON_SIZE: i32 = 32;
pub const MAX_ICON_SIZE: i32 = 64;

pub const SHADOW_SIZE_X: i32 = 3;
pub const SHADOW_SIZE_Y: i32 = 5;
pub const THREE_DEE_OFFSET: i32 = 15;
pub const MULTIPLE_OFFSET: i32 = 10;

pub const INNER_BORDER_OFFSET: i32 = 5;

/// Background color (theme token).
pub const BG_COLOR: &str = d2_color::N7;
/// Foreground color (theme token).
pub const FG_COLOR: &str = d2_color::N1;

pub const MIN_ARROWHEAD_STROKE_WIDTH: f64 = 2.0;
pub const ARROWHEAD_PADDING: f64 = 2.0;

pub const CONNECTION_ICON_LABEL_GAP: f64 = 8.0;

// Class layout constants
pub const PREFIX_PADDING: i32 = 10;
pub const PREFIX_WIDTH: i32 = 20;
pub const CENTER_PADDING: i32 = 50;
pub const VERTICAL_PADDING: i32 = 20;

// SQL table layout constants
pub const NAME_PADDING: i32 = 10;
pub const TYPE_PADDING: i32 = 20;
pub const CONSTRAINT_PADDING: i32 = 20;
pub const HEADER_PADDING: i32 = 10;
pub const HEADER_FONT_ADD: i32 = 4;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub sketch: Option<bool>,
    pub theme_id: Option<i64>,
    pub dark_theme_id: Option<i64>,
    pub pad: Option<i64>,
    pub center: Option<bool>,
    pub layout_engine: Option<String>,
    pub theme_overrides: Option<d2_themes::ThemeOverrides>,
    pub dark_theme_overrides: Option<d2_themes::ThemeOverrides>,
    pub data: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Point
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

// ---------------------------------------------------------------------------
// Shape type constants
// ---------------------------------------------------------------------------

pub const SHAPE_RECTANGLE: &str = "rectangle";
pub const SHAPE_SQUARE: &str = "square";
pub const SHAPE_PAGE: &str = "page";
pub const SHAPE_PARALLELOGRAM: &str = "parallelogram";
pub const SHAPE_DOCUMENT: &str = "document";
pub const SHAPE_CYLINDER: &str = "cylinder";
pub const SHAPE_QUEUE: &str = "queue";
pub const SHAPE_PACKAGE: &str = "package";
pub const SHAPE_STEP: &str = "step";
pub const SHAPE_CALLOUT: &str = "callout";
pub const SHAPE_STORED_DATA: &str = "stored_data";
pub const SHAPE_PERSON: &str = "person";
pub const SHAPE_C4_PERSON: &str = "c4-person";
pub const SHAPE_DIAMOND: &str = "diamond";
pub const SHAPE_OVAL: &str = "oval";
pub const SHAPE_CIRCLE: &str = "circle";
pub const SHAPE_HEXAGON: &str = "hexagon";
pub const SHAPE_CLOUD: &str = "cloud";
pub const SHAPE_TEXT: &str = "text";
pub const SHAPE_CODE: &str = "code";
pub const SHAPE_CLASS: &str = "class";
pub const SHAPE_SQL_TABLE: &str = "sql_table";
pub const SHAPE_IMAGE: &str = "image";
pub const SHAPE_SEQUENCE_DIAGRAM: &str = "sequence_diagram";
pub const SHAPE_HIERARCHY: &str = "hierarchy";

pub const SHAPES: &[&str] = &[
    SHAPE_RECTANGLE,
    SHAPE_SQUARE,
    SHAPE_PAGE,
    SHAPE_PARALLELOGRAM,
    SHAPE_DOCUMENT,
    SHAPE_CYLINDER,
    SHAPE_QUEUE,
    SHAPE_PACKAGE,
    SHAPE_STEP,
    SHAPE_CALLOUT,
    SHAPE_STORED_DATA,
    SHAPE_PERSON,
    SHAPE_C4_PERSON,
    SHAPE_DIAMOND,
    SHAPE_OVAL,
    SHAPE_CIRCLE,
    SHAPE_HEXAGON,
    SHAPE_CLOUD,
    SHAPE_TEXT,
    SHAPE_CODE,
    SHAPE_CLASS,
    SHAPE_SQL_TABLE,
    SHAPE_IMAGE,
    SHAPE_SEQUENCE_DIAGRAM,
    SHAPE_HIERARCHY,
];

/// Check if a string is a recognized shape type.
///
/// Empty string defaults to rectangle and returns `true`.
pub fn is_shape(s: &str) -> bool {
    if s.is_empty() {
        return true; // default shape is rectangle
    }
    SHAPES.iter().any(|shape| shape.eq_ignore_ascii_case(s))
}

// ---------------------------------------------------------------------------
// Shape type to internal shape type mapping
// ---------------------------------------------------------------------------

/// Internal shape type constants (from Go `lib/shape`).
pub mod shape_type {
    pub const SQUARE: &str = "Square";
    pub const REAL_SQUARE: &str = "RealSquare";
    pub const PARALLELOGRAM: &str = "Parallelogram";
    pub const DOCUMENT: &str = "Document";
    pub const CYLINDER: &str = "Cylinder";
    pub const QUEUE: &str = "Queue";
    pub const PAGE: &str = "Page";
    pub const PACKAGE: &str = "Package";
    pub const STEP: &str = "Step";
    pub const CALLOUT: &str = "Callout";
    pub const STORED_DATA: &str = "StoredData";
    pub const PERSON: &str = "Person";
    pub const C4_PERSON: &str = "C4Person";
    pub const DIAMOND: &str = "Diamond";
    pub const OVAL: &str = "Oval";
    pub const CIRCLE: &str = "Circle";
    pub const HEXAGON: &str = "Hexagon";
    pub const CLOUD: &str = "Cloud";
    pub const TABLE: &str = "Table";
    pub const CLASS: &str = "Class";
    pub const TEXT: &str = "Text";
    pub const CODE: &str = "Code";
    pub const IMAGE: &str = "Image";
}

/// Map a DSL shape name to an internal shape type string.
///
/// Empty string maps to `"Square"` (the default rectangle).
pub fn dsl_shape_to_shape_type(dsl: &str) -> &'static str {
    match dsl {
        "" | SHAPE_RECTANGLE => shape_type::SQUARE,
        SHAPE_SQUARE => shape_type::REAL_SQUARE,
        SHAPE_PAGE => shape_type::PAGE,
        SHAPE_PARALLELOGRAM => shape_type::PARALLELOGRAM,
        SHAPE_DOCUMENT => shape_type::DOCUMENT,
        SHAPE_CYLINDER => shape_type::CYLINDER,
        SHAPE_QUEUE => shape_type::QUEUE,
        SHAPE_PACKAGE => shape_type::PACKAGE,
        SHAPE_STEP => shape_type::STEP,
        SHAPE_CALLOUT => shape_type::CALLOUT,
        SHAPE_STORED_DATA => shape_type::STORED_DATA,
        SHAPE_PERSON => shape_type::PERSON,
        SHAPE_C4_PERSON => shape_type::C4_PERSON,
        SHAPE_DIAMOND => shape_type::DIAMOND,
        SHAPE_OVAL => shape_type::OVAL,
        SHAPE_CIRCLE => shape_type::CIRCLE,
        SHAPE_HEXAGON => shape_type::HEXAGON,
        SHAPE_CLOUD => shape_type::CLOUD,
        SHAPE_TEXT => shape_type::TEXT,
        SHAPE_CODE => shape_type::CODE,
        SHAPE_CLASS => shape_type::CLASS,
        SHAPE_SQL_TABLE => shape_type::TABLE,
        SHAPE_IMAGE => shape_type::IMAGE,
        SHAPE_SEQUENCE_DIAGRAM | SHAPE_HIERARCHY => shape_type::SQUARE,
        _ => shape_type::SQUARE,
    }
}

/// Map an internal shape type to a DSL shape name.
pub fn shape_type_to_dsl_shape(st: &str) -> &'static str {
    match st {
        shape_type::SQUARE => SHAPE_RECTANGLE,
        shape_type::REAL_SQUARE => SHAPE_SQUARE,
        shape_type::PAGE => SHAPE_PAGE,
        shape_type::PARALLELOGRAM => SHAPE_PARALLELOGRAM,
        shape_type::DOCUMENT => SHAPE_DOCUMENT,
        shape_type::CYLINDER => SHAPE_CYLINDER,
        shape_type::QUEUE => SHAPE_QUEUE,
        shape_type::PACKAGE => SHAPE_PACKAGE,
        shape_type::STEP => SHAPE_STEP,
        shape_type::CALLOUT => SHAPE_CALLOUT,
        shape_type::STORED_DATA => SHAPE_STORED_DATA,
        shape_type::PERSON => SHAPE_PERSON,
        shape_type::C4_PERSON => SHAPE_C4_PERSON,
        shape_type::DIAMOND => SHAPE_DIAMOND,
        shape_type::OVAL => SHAPE_OVAL,
        shape_type::CIRCLE => SHAPE_CIRCLE,
        shape_type::HEXAGON => SHAPE_HEXAGON,
        shape_type::CLOUD => SHAPE_CLOUD,
        shape_type::TABLE => SHAPE_SQL_TABLE,
        shape_type::CLASS => SHAPE_CLASS,
        shape_type::TEXT => SHAPE_TEXT,
        shape_type::CODE => SHAPE_CODE,
        shape_type::IMAGE => SHAPE_IMAGE,
        _ => SHAPE_RECTANGLE,
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Text {
    pub label: String,
    pub font_size: i32,
    pub font_family: String,
    pub language: String,
    pub color: String,

    pub italic: bool,
    pub bold: bool,
    pub underline: bool,

    pub label_width: i32,
    pub label_height: i32,
    pub label_fill: String,
}

// ---------------------------------------------------------------------------
// MText (measured text, used in Class and SQLTable)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MText {
    pub text: String,
    pub font_size: i32,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_underline: bool,
    pub language: String,
    pub shape: String,
    pub dimensions: Option<TextDimensions>,
}

impl MText {
    pub fn get_color(&self, is_italic: bool) -> &str {
        if is_italic {
            d2_color::N2
        } else {
            d2_color::N1
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TextDimensions {
    pub width: i32,
    pub height: i32,
}

impl TextDimensions {
    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

// ---------------------------------------------------------------------------
// Arrowhead
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Arrowhead {
    None,
    Arrow,
    UnfilledTriangle,
    Triangle,
    Diamond,
    FilledDiamond,
    Circle,
    FilledCircle,
    Cross,
    Box_,
    FilledBox,
    Line,
    CfOne,
    CfMany,
    CfOneRequired,
    CfManyRequired,
}

impl Default for Arrowhead {
    fn default() -> Self {
        Self::None
    }
}

impl Arrowhead {
    pub const DEFAULT: Arrowhead = Arrowhead::Triangle;

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Arrow => "arrow",
            Self::UnfilledTriangle => "unfilled-triangle",
            Self::Triangle => "triangle",
            Self::Diamond => "diamond",
            Self::FilledDiamond => "filled-diamond",
            Self::Circle => "circle",
            Self::FilledCircle => "filled-circle",
            Self::Cross => "cross",
            Self::Box_ => "box",
            Self::FilledBox => "filled-box",
            Self::Line => "line",
            Self::CfOne => "cf-one",
            Self::CfMany => "cf-many",
            Self::CfOneRequired => "cf-one-required",
            Self::CfManyRequired => "cf-many-required",
        }
    }

    pub fn from_str_val(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "arrow" => Self::Arrow,
            "unfilled-triangle" => Self::UnfilledTriangle,
            "triangle" => Self::Triangle,
            "diamond" => Self::Diamond,
            "filled-diamond" => Self::FilledDiamond,
            "circle" => Self::Circle,
            "filled-circle" => Self::FilledCircle,
            "cross" => Self::Cross,
            "box" => Self::Box_,
            "filled-box" => Self::FilledBox,
            "line" => Self::Line,
            "cf-one" => Self::CfOne,
            "cf-many" => Self::CfMany,
            "cf-one-required" => Self::CfOneRequired,
            "cf-many-required" => Self::CfManyRequired,
            _ => Self::DEFAULT,
        }
    }

    /// Convert an arrowhead type string with optional filled flag to an Arrowhead.
    pub fn to_arrowhead(arrowhead_type: &str, filled: Option<bool>) -> Self {
        match arrowhead_type {
            "diamond" => {
                if filled == Some(true) {
                    Self::FilledDiamond
                } else {
                    Self::Diamond
                }
            }
            "circle" => {
                if filled == Some(true) {
                    Self::FilledCircle
                } else {
                    Self::Circle
                }
            }
            "none" => Self::None,
            "arrow" => Self::Arrow,
            "triangle" => {
                if filled == Some(false) {
                    Self::UnfilledTriangle
                } else {
                    Self::Triangle
                }
            }
            "cross" => Self::Cross,
            "box" => {
                if filled == Some(true) {
                    Self::FilledBox
                } else {
                    Self::Box_
                }
            }
            "cf-one" => Self::CfOne,
            "cf-many" => Self::CfMany,
            "cf-one-required" => Self::CfOneRequired,
            "cf-many-required" => Self::CfManyRequired,
            _ => {
                // Default arrowhead is Triangle; respect filled=false
                if Self::DEFAULT == Self::Triangle && filled == Some(false) {
                    Self::UnfilledTriangle
                } else {
                    Self::DEFAULT
                }
            }
        }
    }

    /// Return the (width, height) dimensions of the arrowhead for the given stroke width.
    pub fn dimensions(&self, stroke_width: f64) -> (f64, f64) {
        let (base_w, base_h, w_mul, h_mul) = match self {
            Self::Arrow => (4.0, 4.0, 4.0, 4.0),
            Self::Triangle => (4.0, 4.0, 3.0, 4.0),
            Self::UnfilledTriangle => (7.0, 7.0, 3.0, 4.0),
            Self::Line => (0.0, 0.0, 5.0, 8.0),
            Self::FilledDiamond => (11.0, 7.0, 5.5, 3.5),
            Self::Diamond => (11.0, 9.0, 5.5, 4.5),
            Self::Cross => (7.0, 7.0, 5.0, 5.0),
            Self::FilledCircle | Self::Circle => (8.0, 8.0, 5.0, 5.0),
            Self::FilledBox | Self::Box_ => (6.0, 6.0, 5.0, 5.0),
            Self::CfOne | Self::CfMany | Self::CfOneRequired | Self::CfManyRequired => {
                (9.0, 9.0, 4.5, 4.5)
            }
            Self::None => (0.0, 0.0, 0.0, 0.0),
        };
        let clipped = stroke_width.max(MIN_ARROWHEAD_STROKE_WIDTH);
        (base_w + clipped * w_mul, base_h + clipped * h_mul)
    }
}

/// Valid arrowhead shape values for DSL parsing.
pub const ARROWHEADS: &[&str] = &[
    "none",
    "arrow",
    "triangle",
    "diamond",
    "circle",
    "box",
    "cf-one",
    "cf-many",
    "cf-one-required",
    "cf-many-required",
    "cross",
];

impl std::fmt::Display for Arrowhead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Class types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Class {
    pub fields: Vec<ClassField>,
    pub methods: Vec<ClassMethod>,
}

#[derive(Debug, Clone, Default)]
pub struct ClassField {
    pub name: String,
    pub type_: String,
    pub visibility: String,
    pub underline: bool,
}

impl ClassField {
    pub fn text(&self, font_size: i32) -> MText {
        MText {
            text: format!("{}{}", self.name, self.type_),
            font_size,
            is_bold: false,
            is_italic: false,
            is_underline: self.underline,
            shape: "class".to_owned(),
            ..Default::default()
        }
    }

    pub fn visibility_token(&self) -> &str {
        match self.visibility.as_str() {
            "protected" => "#",
            "private" => "-",
            _ => "+",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClassMethod {
    pub name: String,
    pub return_: String,
    pub visibility: String,
    pub underline: bool,
}

impl ClassMethod {
    pub fn text(&self, font_size: i32) -> MText {
        MText {
            text: format!("{}{}", self.name, self.return_),
            font_size,
            is_bold: false,
            is_italic: false,
            is_underline: self.underline,
            shape: "class".to_owned(),
            ..Default::default()
        }
    }

    pub fn visibility_token(&self) -> &str {
        match self.visibility.as_str() {
            "protected" => "#",
            "private" => "-",
            _ => "+",
        }
    }
}

// ---------------------------------------------------------------------------
// SQL Table types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SQLTable {
    pub columns: Vec<SQLColumn>,
}

#[derive(Debug, Clone, Default)]
pub struct SQLColumn {
    pub name: Text,
    pub type_: Text,
    pub constraint: Vec<String>,
    pub reference: String,
}

impl SQLColumn {
    pub fn texts(&self, font_size: i32) -> Vec<MText> {
        vec![
            MText {
                text: self.name.label.clone(),
                font_size,
                shape: "sql_table".to_owned(),
                ..Default::default()
            },
            MText {
                text: self.type_.label.clone(),
                font_size,
                shape: "sql_table".to_owned(),
                ..Default::default()
            },
            MText {
                text: self.constraint_abbr(),
                font_size,
                shape: "sql_table".to_owned(),
                ..Default::default()
            },
        ]
    }

    pub fn constraint_abbr(&self) -> String {
        let abbrs: Vec<&str> = self
            .constraint
            .iter()
            .map(|c| match c.as_str() {
                "primary_key" => "PK",
                "foreign_key" => "FK",
                "unique" => "UNQ",
                other => other,
            })
            .collect();
        abbrs.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Legend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Legend {
    pub label: String,
    pub shapes: Vec<Shape>,
    pub connections: Vec<Connection>,
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Shape {
    pub id: String,
    pub type_: String,

    pub classes: Vec<String>,

    pub pos: Point,
    pub width: i32,
    pub height: i32,

    pub opacity: f64,
    pub stroke_dash: f64,
    pub stroke_width: i32,

    pub border_radius: i32,

    pub fill: String,
    pub fill_pattern: String,
    pub stroke: String,

    pub animated: bool,
    pub shadow: bool,
    pub three_dee: bool,
    pub multiple: bool,
    pub double_border: bool,

    pub tooltip: String,
    pub link: String,
    pub pretty_link: String,
    pub icon: Option<String>,
    pub icon_border_radius: i32,
    pub icon_position: String,

    /// Whether the shape should allow shapes behind it to bleed through.
    pub blend: bool,

    pub class: Class,
    pub sql_table: SQLTable,

    pub content_aspect_ratio: Option<f64>,

    pub text: Text,

    pub label_position: String,
    pub tooltip_position: String,

    pub z_index: i32,
    pub level: i32,

    pub primary_accent_color: String,
    pub secondary_accent_color: String,
    pub neutral_accent_color: String,
}

impl Shape {
    pub fn get_font_color(&self) -> &str {
        if self.type_ == SHAPE_CLASS || self.type_ == SHAPE_SQL_TABLE {
            if !d2_color::is_theme_color(&self.text.color) {
                return &self.text.color;
            }
            return &self.stroke;
        }
        if !self.text.color.is_empty() {
            return &self.text.color;
        }
        d2_color::N1
    }

    /// Set the shape type, normalizing synonyms.
    pub fn set_type(&mut self, t: &str) {
        let lower = t.to_ascii_lowercase();
        if lower == SHAPE_CIRCLE {
            self.type_ = SHAPE_OVAL.to_owned();
        } else if lower == SHAPE_SQUARE {
            self.type_ = SHAPE_RECTANGLE.to_owned();
        } else {
            self.type_ = lower;
        }
    }

    pub fn get_z_index(&self) -> i32 {
        self.z_index
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }
}

/// Create a new base shape with default styling.
pub fn base_shape() -> Shape {
    Shape {
        opacity: 1.0,
        stroke_dash: 0.0,
        stroke_width: 2,
        text: Text {
            bold: true,
            font_family: "DEFAULT".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Connection {
    pub id: String,

    pub classes: Vec<String>,

    pub src: String,
    pub src_arrow: Arrowhead,
    pub src_label: Option<Text>,

    pub dst: String,
    pub dst_arrow: Arrowhead,
    pub dst_label: Option<Text>,

    pub opacity: f64,
    pub stroke_dash: f64,
    pub stroke_width: i32,
    pub stroke: String,
    pub fill: String,
    pub border_radius: f64,

    pub text: Text,
    pub label_position: String,
    pub label_percentage: f64,

    pub link: String,
    pub pretty_link: String,

    pub route: Vec<d2_geo::Point>,
    pub is_curve: bool,

    pub animated: bool,
    pub tooltip: String,
    pub icon: Option<String>,
    pub icon_position: String,
    pub icon_border_radius: f64,

    pub z_index: i32,
}

impl Connection {
    pub fn get_font_color(&self) -> &str {
        if !self.text.color.is_empty() {
            return &self.text.color;
        }
        d2_color::N1
    }

    pub fn get_z_index(&self) -> i32 {
        self.z_index
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }
}

/// Create a new base connection with default styling.
pub fn base_connection() -> Connection {
    Connection {
        src_arrow: Arrowhead::None,
        dst_arrow: Arrowhead::None,
        route: Vec::new(),
        opacity: 1.0,
        stroke_dash: 0.0,
        stroke_width: 2,
        border_radius: 10.0,
        text: Text {
            italic: true,
            font_family: "DEFAULT".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Diagram
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Diagram {
    pub name: String,
    pub config: Option<Config>,
    pub is_folder_only: bool,
    pub description: String,
    pub font_family: Option<String>,
    pub mono_font_family: Option<String>,

    pub shapes: Vec<Shape>,
    pub connections: Vec<Connection>,

    pub root: Shape,
    pub legend: Option<Legend>,

    pub layers: Vec<Diagram>,
    pub scenarios: Vec<Diagram>,
    pub steps: Vec<Diagram>,
}

impl Diagram {
    /// Create a new diagram with the default root shape.
    pub fn new() -> Self {
        Self {
            root: Shape {
                fill: BG_COLOR.to_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Navigate to a nested board by path.
    pub fn get_board(&self, board_path: &[&str]) -> Option<&Diagram> {
        if board_path.is_empty() {
            return Some(self);
        }

        let head = board_path[0];

        if board_path.len() == 1 && self.name == head {
            return Some(self);
        }

        match head {
            "layers" if board_path.len() >= 2 => {
                for b in &self.layers {
                    if b.name == board_path[1] {
                        return b.get_board(&board_path[2..]);
                    }
                }
            }
            "scenarios" if board_path.len() >= 2 => {
                for b in &self.scenarios {
                    if b.name == board_path[1] {
                        return b.get_board(&board_path[2..]);
                    }
                }
            }
            "steps" if board_path.len() >= 2 => {
                for b in &self.steps {
                    if b.name == board_path[1] {
                        return b.get_board(&board_path[2..]);
                    }
                }
            }
            _ => {}
        }

        // Also try matching directly by name
        for b in &self.layers {
            if b.name == head {
                return b.get_board(&board_path[1..]);
            }
        }
        for b in &self.scenarios {
            if b.name == head {
                return b.get_board(&board_path[1..]);
            }
        }
        for b in &self.steps {
            if b.name == head {
                return b.get_board(&board_path[1..]);
            }
        }
        None
    }

    /// Check if any shape in the diagram (including nested boards) satisfies a condition.
    pub fn has_shape<F>(&self, condition: &F) -> bool
    where
        F: Fn(&Shape) -> bool,
    {
        for d in &self.layers {
            if d.has_shape(condition) {
                return true;
            }
        }
        for d in &self.scenarios {
            if d.has_shape(condition) {
                return true;
            }
        }
        for d in &self.steps {
            if d.has_shape(condition) {
                return true;
            }
        }
        for s in &self.shapes {
            if condition(s) {
                return true;
            }
        }
        false
    }

    /// Compute a hash ID for the diagram, prefixed with "d2-".
    pub fn hash_id(&self, salt: Option<&str>) -> String {
        let mut hasher = DefaultHasher::new();
        // Hash shape IDs and types
        for s in &self.shapes {
            s.id.hash(&mut hasher);
            s.type_.hash(&mut hasher);
            s.text.label.hash(&mut hasher);
        }
        // Hash connection IDs
        for c in &self.connections {
            c.id.hash(&mut hasher);
            c.src.hash(&mut hasher);
            c.dst.hash(&mut hasher);
        }
        // Hash root
        self.root.fill.hash(&mut hasher);

        if let Some(s) = salt {
            s.hash(&mut hasher);
        }

        // Hash nested
        for d in &self.layers {
            d.name.hash(&mut hasher);
        }
        for d in &self.scenarios {
            d.name.hash(&mut hasher);
        }
        for d in &self.steps {
            d.name.hash(&mut hasher);
        }

        format!("d2-{}", hasher.finish() as u32)
    }

    /// Compute the axis-aligned bounding box of all shapes and connections.
    ///
    /// Returns `(top_left, bottom_right)`.
    pub fn bounding_box(&self) -> (Point, Point) {
        if self.shapes.is_empty() {
            return (Point::new(0, 0), Point::new(0, 0));
        }

        let mut x1 = i32::MAX;
        let mut y1 = i32::MAX;
        let mut x2 = i32::MIN;
        let mut y2 = i32::MIN;

        for s in &self.shapes {
            let half_stroke = (s.stroke_width as f64 / 2.0).ceil() as i32;
            x1 = x1.min(s.pos.x - half_stroke);
            y1 = y1.min(s.pos.y - half_stroke);
            x2 = x2.max(s.pos.x + s.width + half_stroke);
            y2 = y2.max(s.pos.y + s.height + half_stroke);

            if s.shadow {
                y2 = y2.max(s.pos.y + s.height + half_stroke + SHADOW_SIZE_Y);
                x2 = x2.max(s.pos.x + s.width + half_stroke + SHADOW_SIZE_X);
            }

            if s.three_dee {
                let offset_y = if s.type_ == SHAPE_HEXAGON {
                    THREE_DEE_OFFSET / 2
                } else {
                    THREE_DEE_OFFSET
                };
                y1 = y1.min(s.pos.y - offset_y - s.stroke_width);
                x2 = x2.max(s.pos.x + THREE_DEE_OFFSET + s.width + s.stroke_width);
            }
            if s.multiple {
                y1 = y1.min(s.pos.y - MULTIPLE_OFFSET - s.stroke_width);
                x2 = x2.max(s.pos.x + MULTIPLE_OFFSET + s.width + s.stroke_width);
            }
        }

        for c in &self.connections {
            for point in &c.route {
                let half_stroke = (c.stroke_width as f64 / 2.0).ceil() as i32;
                x1 = x1.min(point.x.floor() as i32 - half_stroke);
                y1 = y1.min(point.y.floor() as i32 - half_stroke);
                x2 = x2.max(point.x.ceil() as i32 + half_stroke);
                y2 = y2.max(point.y.ceil() as i32 + half_stroke);
            }
        }

        (Point::new(x1, y1), Point::new(x2, y2))
    }

    /// Compute the bounding box including nested layers, scenarios, and steps.
    pub fn nested_bounding_box(&self) -> (Point, Point) {
        let (mut tl, mut br) = self.bounding_box();
        for d in &self.layers {
            let (tl2, br2) = d.nested_bounding_box();
            tl.x = tl.x.min(tl2.x);
            tl.y = tl.y.min(tl2.y);
            br.x = br.x.max(br2.x);
            br.y = br.y.max(br2.y);
        }
        for d in &self.scenarios {
            let (tl2, br2) = d.nested_bounding_box();
            tl.x = tl.x.min(tl2.x);
            tl.y = tl.y.min(tl2.y);
            br.x = br.x.max(br2.x);
            br.y = br.y.max(br2.y);
        }
        for d in &self.steps {
            let (tl2, br2) = d.nested_bounding_box();
            tl.x = tl.x.min(tl2.x);
            tl.y = tl.y.min(tl2.y);
            br.x = br.x.max(br2.x);
            br.y = br.y.max(br2.y);
        }
        (tl, br)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Shape type tests --

    #[test]
    fn is_shape_recognizes_all() {
        for s in SHAPES {
            assert!(is_shape(s), "is_shape should recognize: {s}");
        }
    }

    #[test]
    fn is_shape_case_insensitive() {
        assert!(is_shape("Rectangle"));
        assert!(is_shape("RECTANGLE"));
        assert!(is_shape("rectangle"));
    }

    #[test]
    fn is_shape_empty_is_default() {
        assert!(is_shape(""));
    }

    #[test]
    fn is_shape_rejects_unknown() {
        assert!(!is_shape("nonexistent"));
    }

    // -- DSL shape mapping --

    #[test]
    fn dsl_to_shape_type_mapping() {
        assert_eq!(dsl_shape_to_shape_type(""), shape_type::SQUARE);
        assert_eq!(dsl_shape_to_shape_type(SHAPE_RECTANGLE), shape_type::SQUARE);
        assert_eq!(
            dsl_shape_to_shape_type(SHAPE_SQUARE),
            shape_type::REAL_SQUARE
        );
        assert_eq!(dsl_shape_to_shape_type(SHAPE_OVAL), shape_type::OVAL);
        assert_eq!(dsl_shape_to_shape_type(SHAPE_SQL_TABLE), shape_type::TABLE);
        assert_eq!(dsl_shape_to_shape_type(SHAPE_CLASS), shape_type::CLASS);
        assert_eq!(
            dsl_shape_to_shape_type(SHAPE_SEQUENCE_DIAGRAM),
            shape_type::SQUARE
        );
    }

    #[test]
    fn shape_type_to_dsl_round_trip() {
        // Every shape type should map back to a dsl shape
        let test_cases = &[
            (shape_type::SQUARE, SHAPE_RECTANGLE),
            (shape_type::OVAL, SHAPE_OVAL),
            (shape_type::TABLE, SHAPE_SQL_TABLE),
            (shape_type::CLASS, SHAPE_CLASS),
            (shape_type::IMAGE, SHAPE_IMAGE),
        ];
        for &(st, expected_dsl) in test_cases {
            assert_eq!(shape_type_to_dsl_shape(st), expected_dsl);
        }
    }

    // -- Arrowhead tests --

    #[test]
    fn arrowhead_str_round_trip() {
        let cases = &[
            Arrowhead::None,
            Arrowhead::Arrow,
            Arrowhead::Triangle,
            Arrowhead::UnfilledTriangle,
            Arrowhead::Diamond,
            Arrowhead::FilledDiamond,
            Arrowhead::Circle,
            Arrowhead::FilledCircle,
            Arrowhead::Cross,
            Arrowhead::Box_,
            Arrowhead::FilledBox,
            Arrowhead::Line,
            Arrowhead::CfOne,
            Arrowhead::CfMany,
            Arrowhead::CfOneRequired,
            Arrowhead::CfManyRequired,
        ];
        for ah in cases {
            let s = ah.as_str();
            let parsed = Arrowhead::from_str_val(s);
            assert_eq!(&parsed, ah, "round trip failed for {s}");
        }
    }

    #[test]
    fn to_arrowhead_filled_diamond() {
        assert_eq!(
            Arrowhead::to_arrowhead("diamond", Some(true)),
            Arrowhead::FilledDiamond
        );
        assert_eq!(Arrowhead::to_arrowhead("diamond", None), Arrowhead::Diamond);
    }

    #[test]
    fn to_arrowhead_unfilled_triangle() {
        assert_eq!(
            Arrowhead::to_arrowhead("triangle", Some(false)),
            Arrowhead::UnfilledTriangle
        );
    }

    #[test]
    fn arrowhead_dimensions() {
        let (w, h) = Arrowhead::Triangle.dimensions(2.0);
        assert!(w > 0.0 && h > 0.0);

        let (w0, h0) = Arrowhead::None.dimensions(2.0);
        assert_eq!(w0, 0.0);
        assert_eq!(h0, 0.0);
    }

    // -- Shape set_type --

    #[test]
    fn shape_set_type_normalizes() {
        let mut s = Shape::default();
        s.set_type("circle");
        assert_eq!(s.type_, SHAPE_OVAL);

        s.set_type("square");
        assert_eq!(s.type_, SHAPE_RECTANGLE);

        s.set_type("Diamond");
        assert_eq!(s.type_, SHAPE_DIAMOND);
    }

    // -- Class / SQL Table --

    #[test]
    fn class_field_visibility_token() {
        let f = ClassField {
            visibility: "private".to_owned(),
            ..Default::default()
        };
        assert_eq!(f.visibility_token(), "-");

        let f2 = ClassField {
            visibility: "protected".to_owned(),
            ..Default::default()
        };
        assert_eq!(f2.visibility_token(), "#");

        let f3 = ClassField::default();
        assert_eq!(f3.visibility_token(), "+");
    }

    #[test]
    fn sql_column_constraint_abbr() {
        let col = SQLColumn {
            constraint: vec!["primary_key".to_owned(), "unique".to_owned()],
            ..Default::default()
        };
        assert_eq!(col.constraint_abbr(), "PK, UNQ");
    }

    // -- Diagram tests --

    #[test]
    fn diagram_new_has_bg_fill() {
        let d = Diagram::new();
        assert_eq!(d.root.fill, "N7");
    }

    #[test]
    fn diagram_bounding_box_empty() {
        let d = Diagram::new();
        let (tl, br) = d.bounding_box();
        assert_eq!(tl, Point::new(0, 0));
        assert_eq!(br, Point::new(0, 0));
    }

    #[test]
    fn diagram_bounding_box_single_shape() {
        let mut d = Diagram::new();
        d.shapes.push(Shape {
            pos: Point::new(10, 20),
            width: 100,
            height: 50,
            stroke_width: 2,
            ..Default::default()
        });
        let (tl, br) = d.bounding_box();
        assert_eq!(tl, Point::new(9, 19));
        assert_eq!(br, Point::new(111, 71));
    }

    #[test]
    fn diagram_hash_id_deterministic() {
        let mut d = Diagram::new();
        d.shapes.push(Shape {
            id: "a".to_owned(),
            text: Text {
                label: "hello".to_owned(),
                ..Default::default()
            },
            ..Default::default()
        });
        let h1 = d.hash_id(None);
        let h2 = d.hash_id(None);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("d2-"));
    }

    #[test]
    fn diagram_hash_id_with_salt() {
        let d = Diagram::new();
        let h1 = d.hash_id(None);
        let h2 = d.hash_id(Some("salt"));
        assert_ne!(h1, h2);
    }

    #[test]
    fn diagram_get_board() {
        let mut d = Diagram::new();
        d.name = "root".to_owned();
        d.layers.push(Diagram {
            name: "layer1".to_owned(),
            ..Default::default()
        });

        assert!(d.get_board(&["layers", "layer1"]).is_some());
        assert!(d.get_board(&["layer1"]).is_some());
        assert!(d.get_board(&["nonexistent"]).is_none());
    }

    #[test]
    fn base_shape_defaults() {
        let s = base_shape();
        assert_eq!(s.opacity, 1.0);
        assert_eq!(s.stroke_width, 2);
        assert!(s.text.bold);
    }

    #[test]
    fn base_connection_defaults() {
        let c = base_connection();
        assert_eq!(c.src_arrow, Arrowhead::None);
        assert_eq!(c.dst_arrow, Arrowhead::None);
        assert_eq!(c.opacity, 1.0);
        assert_eq!(c.stroke_width, 2);
        assert!(c.text.italic);
    }
}
