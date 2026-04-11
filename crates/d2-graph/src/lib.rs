//! d2-graph: core graph types for d2 diagram compilation and layout.
//!
//! These types bridge the d2 AST/IR with layout engines and exporters.
//! Ported from Go `d2graph/d2graph.go`.

use d2_geo::{self, Box2D, Point, Segment, Spacing};
use d2_label;
use d2_target;
use d2_themes;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum segment length for edge routing. Shorter segments near arrowheads
/// get extended to avoid rendering artifacts.
pub const MIN_SEGMENT_LEN: f64 = 10.0;

/// Default padding around container contents.
pub const DEFAULT_PADDING: f64 = 30.0;

// ---------------------------------------------------------------------------
// Direction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Direction {
    pub value: String,
}

// ---------------------------------------------------------------------------
// Dimensions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct Dimensions {
    pub width: i32,
    pub height: i32,
}

// ---------------------------------------------------------------------------
// Style (scalar wrapper)
// ---------------------------------------------------------------------------

/// A single style value from the DSL.
#[derive(Debug, Clone, Default)]
pub struct ScalarValue {
    pub value: String,
}

/// Style properties that can be set on objects and edges.
#[derive(Debug, Clone, Default)]
pub struct Style {
    pub opacity: Option<ScalarValue>,
    pub stroke: Option<ScalarValue>,
    pub fill: Option<ScalarValue>,
    pub fill_pattern: Option<ScalarValue>,
    pub stroke_dash: Option<ScalarValue>,
    pub stroke_width: Option<ScalarValue>,
    pub shadow: Option<ScalarValue>,
    pub three_dee: Option<ScalarValue>,
    pub multiple: Option<ScalarValue>,
    pub border_radius: Option<ScalarValue>,
    pub font_color: Option<ScalarValue>,
    pub font_size: Option<ScalarValue>,
    pub italic: Option<ScalarValue>,
    pub bold: Option<ScalarValue>,
    pub underline: Option<ScalarValue>,
    pub font: Option<ScalarValue>,
    pub double_border: Option<ScalarValue>,
    pub animated: Option<ScalarValue>,
    pub filled: Option<ScalarValue>,
    pub text_transform: Option<ScalarValue>,
}

impl Style {
    /// Initialize a style field to `Some(ScalarValue::default())` so `apply` can set it.
    pub fn init_field(&mut self, key: &str) {
        match key {
            "opacity" => {
                if self.opacity.is_none() {
                    self.opacity = Some(ScalarValue::default());
                }
            }
            "stroke" => {
                if self.stroke.is_none() {
                    self.stroke = Some(ScalarValue::default());
                }
            }
            "fill" => {
                if self.fill.is_none() {
                    self.fill = Some(ScalarValue::default());
                }
            }
            "fill-pattern" => {
                if self.fill_pattern.is_none() {
                    self.fill_pattern = Some(ScalarValue::default());
                }
            }
            "stroke-width" => {
                if self.stroke_width.is_none() {
                    self.stroke_width = Some(ScalarValue::default());
                }
            }
            "stroke-dash" => {
                if self.stroke_dash.is_none() {
                    self.stroke_dash = Some(ScalarValue::default());
                }
            }
            "border-radius" => {
                if self.border_radius.is_none() {
                    self.border_radius = Some(ScalarValue::default());
                }
            }
            "shadow" => {
                if self.shadow.is_none() {
                    self.shadow = Some(ScalarValue::default());
                }
            }
            "3d" => {
                if self.three_dee.is_none() {
                    self.three_dee = Some(ScalarValue::default());
                }
            }
            "multiple" => {
                if self.multiple.is_none() {
                    self.multiple = Some(ScalarValue::default());
                }
            }
            "font" => {
                if self.font.is_none() {
                    self.font = Some(ScalarValue::default());
                }
            }
            "font-size" => {
                if self.font_size.is_none() {
                    self.font_size = Some(ScalarValue::default());
                }
            }
            "font-color" => {
                if self.font_color.is_none() {
                    self.font_color = Some(ScalarValue::default());
                }
            }
            "animated" => {
                if self.animated.is_none() {
                    self.animated = Some(ScalarValue::default());
                }
            }
            "bold" => {
                if self.bold.is_none() {
                    self.bold = Some(ScalarValue::default());
                }
            }
            "italic" => {
                if self.italic.is_none() {
                    self.italic = Some(ScalarValue::default());
                }
            }
            "underline" => {
                if self.underline.is_none() {
                    self.underline = Some(ScalarValue::default());
                }
            }
            "filled" => {
                if self.filled.is_none() {
                    self.filled = Some(ScalarValue::default());
                }
            }
            "double-border" => {
                if self.double_border.is_none() {
                    self.double_border = Some(ScalarValue::default());
                }
            }
            "text-transform" => {
                if self.text_transform.is_none() {
                    self.text_transform = Some(ScalarValue::default());
                }
            }
            _ => {}
        }
    }

    /// Apply a style key-value pair, validating the value.
    pub fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "opacity" => {
                if let Some(s) = self.opacity.as_mut() {
                    let f: f64 = value.parse().map_err(|_| {
                        "expected \"opacity\" to be a number between 0.0 and 1.0".to_string()
                    })?;
                    if !(0.0..=1.0).contains(&f) {
                        return Err(
                            "expected \"opacity\" to be a number between 0.0 and 1.0".to_string()
                        );
                    }
                    s.value = value.to_string();
                }
            }
            "stroke" => {
                if let Some(s) = self.stroke.as_mut() {
                    s.value = value.to_string();
                }
            }
            "fill" => {
                if let Some(s) = self.fill.as_mut() {
                    s.value = value.to_string();
                }
            }
            "fill-pattern" => {
                if let Some(s) = self.fill_pattern.as_mut() {
                    let patterns = ["none", "dots", "lines", "grain", "paper"];
                    if !patterns.contains(&value.to_lowercase().as_str()) {
                        return Err(format!(
                            "expected \"fill-pattern\" to be one of: {}",
                            patterns.join(", ")
                        ));
                    }
                    s.value = value.to_string();
                }
            }
            "stroke-width" => {
                if let Some(s) = self.stroke_width.as_mut() {
                    let v: i32 = value.parse().map_err(|_| {
                        "expected \"stroke-width\" to be a number between 0 and 15".to_string()
                    })?;
                    if !(0..=15).contains(&v) {
                        return Err(
                            "expected \"stroke-width\" to be a number between 0 and 15".to_string()
                        );
                    }
                    s.value = value.to_string();
                }
            }
            "stroke-dash" => {
                if let Some(s) = self.stroke_dash.as_mut() {
                    let v: i32 = value.parse().map_err(|_| {
                        "expected \"stroke-dash\" to be a number between 0 and 10".to_string()
                    })?;
                    if !(0..=10).contains(&v) {
                        return Err(
                            "expected \"stroke-dash\" to be a number between 0 and 10".to_string()
                        );
                    }
                    s.value = value.to_string();
                }
            }
            "border-radius" => {
                if let Some(s) = self.border_radius.as_mut() {
                    let v: i32 = value.parse().map_err(|_| {
                        "expected \"border-radius\" to be a number >= 0".to_string()
                    })?;
                    if v < 0 {
                        return Err("expected \"border-radius\" to be a number >= 0".to_string());
                    }
                    s.value = value.to_string();
                }
            }
            "shadow" | "3d" | "multiple" | "animated" | "bold" | "italic" | "underline"
            | "filled" | "double-border" => {
                let target = match key {
                    "shadow" => self.shadow.as_mut(),
                    "3d" => self.three_dee.as_mut(),
                    "multiple" => self.multiple.as_mut(),
                    "animated" => self.animated.as_mut(),
                    "bold" => self.bold.as_mut(),
                    "italic" => self.italic.as_mut(),
                    "underline" => self.underline.as_mut(),
                    "filled" => self.filled.as_mut(),
                    "double-border" => self.double_border.as_mut(),
                    _ => None,
                };
                if let Some(s) = target {
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("expected \"{}\" to be true or false", key))?;
                    s.value = value.to_string();
                }
            }
            "font" => {
                if let Some(s) = self.font.as_mut() {
                    s.value = value.to_string();
                }
            }
            "font-size" => {
                if let Some(s) = self.font_size.as_mut() {
                    let v: i32 = value.parse().map_err(|_| {
                        "expected \"font-size\" to be a number between 8 and 100".to_string()
                    })?;
                    if !(8..=100).contains(&v) {
                        return Err(
                            "expected \"font-size\" to be a number between 8 and 100".to_string()
                        );
                    }
                    s.value = value.to_string();
                }
            }
            "font-color" => {
                if let Some(s) = self.font_color.as_mut() {
                    s.value = value.to_string();
                }
            }
            "text-transform" => {
                if let Some(s) = self.text_transform.as_mut() {
                    let vals = ["none", "uppercase", "lowercase", "capitalize"];
                    if !vals.contains(&value) {
                        return Err(format!(
                            "expected \"text-transform\" to be one of: {}",
                            vals.join(", ")
                        ));
                    }
                    s.value = value.to_string();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Icon-specific style overrides.
#[derive(Debug, Clone, Default)]
pub struct IconStyle {
    pub border_radius: Option<ScalarValue>,
}

// ---------------------------------------------------------------------------
// Label
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Label {
    pub value: String,
    pub map_key: Option<()>, // simplified: just presence/absence
}

// ---------------------------------------------------------------------------
// MText (measured text associated with objects/edges)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MText {
    pub text: String,
    pub font_size: i32,
    pub is_bold: bool,
    pub is_italic: bool,
    pub dimensions: Dimensions,
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

// ---------------------------------------------------------------------------
// Arrowhead info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ArrowheadInfo {
    pub label: Label,
    pub label_dimensions: Dimensions,
    pub style: Style,
    pub shape: Option<String>,
    pub filled: Option<bool>,
}

impl ArrowheadInfo {
    /// Convert to a target arrowhead type.
    pub fn to_arrowhead(&self) -> d2_target::Arrowhead {
        let shape_str = self.shape.as_deref().unwrap_or("");
        d2_target::Arrowhead::to_arrowhead(shape_str, self.filled)
    }
}

// ---------------------------------------------------------------------------
// ObjId
// ---------------------------------------------------------------------------

/// Unique object identifier within a graph. Uses an index for efficient lookup.
pub type ObjId = usize;

// ---------------------------------------------------------------------------
// Object
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Object {
    pub id: String,
    pub abs_id: String,

    pub label: Label,
    pub shape: ScalarValue,
    pub direction: Direction,
    pub language: String,

    pub top_left: Point,
    pub width: f64,
    pub height: f64,

    /// Bounding box, updated during layout.
    pub box_: Box2D,

    pub parent: Option<ObjId>,
    pub children: Vec<ObjId>,
    pub children_array: Vec<ObjId>,

    pub style: Style,
    pub icon_style: IconStyle,

    pub icon: Option<String>,
    pub icon_position: Option<String>,
    pub label_position: Option<String>,
    pub tooltip_position: Option<String>,
    pub label_dimensions: Dimensions,

    pub tooltip: Option<ScalarValue>,
    pub link: Option<ScalarValue>,

    pub class: Option<d2_target::Class>,
    pub sql_table: Option<d2_target::SQLTable>,
    pub content_aspect_ratio: Option<f64>,

    // Attributes from Go's Attributes struct
    pub width_attr: Option<ScalarValue>,
    pub height_attr: Option<ScalarValue>,
    pub top: Option<ScalarValue>,
    pub left: Option<ScalarValue>,
    pub near_key: Option<String>,
    pub constraint: Vec<String>,
    pub grid_rows: Option<ScalarValue>,
    pub grid_columns: Option<ScalarValue>,
    pub grid_gap: Option<ScalarValue>,
    pub vertical_gap: Option<ScalarValue>,
    pub horizontal_gap: Option<ScalarValue>,

    pub z_index: i32,
    pub classes: Vec<String>,
}

impl Default for Object {
    fn default() -> Self {
        Self {
            id: String::new(),
            abs_id: String::new(),
            label: Label::default(),
            shape: ScalarValue {
                value: "rectangle".to_owned(),
            },
            direction: Direction::default(),
            language: String::new(),
            top_left: Point::new(0.0, 0.0),
            width: 0.0,
            height: 0.0,
            box_: Box2D::new(Point::new(0.0, 0.0), 0.0, 0.0),
            parent: None,
            children: Vec::new(),
            children_array: Vec::new(),
            style: Style::default(),
            icon_style: IconStyle::default(),
            icon: None,
            icon_position: None,
            label_position: None,
            tooltip_position: None,
            label_dimensions: Dimensions::default(),
            tooltip: None,
            link: None,
            class: None,
            sql_table: None,
            content_aspect_ratio: None,
            width_attr: None,
            height_attr: None,
            top: None,
            left: None,
            near_key: None,
            constraint: Vec::new(),
            grid_rows: None,
            grid_columns: None,
            grid_gap: None,
            vertical_gap: None,
            horizontal_gap: None,
            z_index: 0,
            classes: Vec::new(),
        }
    }
}

impl Object {
    /// Absolute ID of the object for export.
    pub fn abs_id(&self) -> &str {
        &self.abs_id
    }

    /// The short ID value (without dotted path).
    pub fn id_val(&self) -> &str {
        &self.id
    }

    /// True if this object has children.
    pub fn is_container(&self) -> bool {
        !self.children_array.is_empty()
    }

    /// Returns nesting level: 0 for root children, 1 for grandchildren, etc.
    pub fn level(&self, graph: &Graph) -> u32 {
        // Go d2graph.Object.Level() returns 1 for top-level (immediate children
        // of root), 2 for grandchildren, etc.
        let mut depth = 1;
        let mut p = self.parent;
        while let Some(pid) = p {
            if pid == graph.root {
                break;
            }
            depth += 1;
            p = graph.objects[pid].parent;
        }
        depth
    }

    /// Return text info for this object.
    /// Matches Go d2graph.Object.Text(): leaf shapes default to bold;
    /// containers/text shapes default to non-bold; explicit style.bold overrides.
    pub fn text(&self) -> MText {
        let is_container = !self.children_array.is_empty();
        let mut is_bold = !is_container && self.shape.value != "text";
        if let Some(v) = self.style.bold.as_ref() {
            is_bold = v.value == "true";
        }
        MText {
            text: self.label.value.clone(),
            font_size: 16, // default
            is_bold,
            is_italic: self
                .style
                .italic
                .as_ref()
                .is_some_and(|v| v.value == "true"),
            dimensions: self.label_dimensions,
        }
    }

    /// Whether the object has a non-empty label.
    pub fn has_label(&self) -> bool {
        !self.label.value.is_empty()
    }

    /// Whether the object has an icon.
    pub fn has_icon(&self) -> bool {
        self.icon.is_some()
    }

    /// Whether the object has an outside bottom label (e.g., image shapes).
    pub fn has_outside_bottom_label(&self) -> bool {
        self.shape.value == d2_target::SHAPE_IMAGE
    }

    /// Update the bounding box from top_left + width + height.
    pub fn update_box(&mut self) {
        self.box_ = Box2D::new(self.top_left, self.width, self.height);
    }

    /// Check if `self` is a descendant of `ancestor_id` in the graph.
    pub fn is_descendant_of(&self, ancestor_id: ObjId, graph: &Graph) -> bool {
        let mut p = self.parent;
        while let Some(pid) = p {
            if pid == ancestor_id {
                return true;
            }
            p = graph.objects[pid].parent;
        }
        false
    }

    /// Get the fill color based on style.
    pub fn get_fill(&self) -> &str {
        if let Some(ref f) = self.style.fill {
            &f.value
        } else {
            d2_color::B6
        }
    }

    /// Get the stroke color based on style and stroke-dash.
    pub fn get_stroke(&self, _stroke_dash: f64) -> &str {
        if let Some(ref s) = self.style.stroke {
            &s.value
        } else {
            d2_color::B1
        }
    }

    /// Get the 3D/multiple modifier adjustments (dx, dy).
    pub fn get_modifier_element_adjustments(&self) -> (f64, f64) {
        let three_dee = self
            .style
            .three_dee
            .as_ref()
            .is_some_and(|v| v.value == "true");
        let multiple = self
            .style
            .multiple
            .as_ref()
            .is_some_and(|v| v.value == "true");
        let mut dx = 0.0;
        let mut dy = 0.0;
        if three_dee {
            dx += d2_target::THREE_DEE_OFFSET as f64;
            dy += d2_target::THREE_DEE_OFFSET as f64;
        }
        if multiple {
            dx += d2_target::MULTIPLE_OFFSET as f64;
            dy += d2_target::MULTIPLE_OFFSET as f64;
        }
        (dx, dy)
    }

    /// Spacing returns (margin, padding) for this object (simplified).
    pub fn spacing(&self) -> (Spacing, Spacing) {
        let zero = Spacing {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        };
        let padding = Spacing {
            top: DEFAULT_PADDING,
            bottom: DEFAULT_PADDING,
            left: DEFAULT_PADDING,
            right: DEFAULT_PADDING,
        };
        if self.is_container() {
            (zero, padding)
        } else {
            (zero, zero)
        }
    }

    /// Trace edge endpoints to the shape boundary.
    /// Returns (new_start_index, new_end_index) after clipping.
    pub fn trace_to_shape_start(&self, points: &[Point], start_index: usize) -> usize {
        // Simplified: clip to bounding box
        for i in (start_index + 1)..points.len() {
            let seg = Segment::new(points[i - 1], points[i]);
            let ints = self.box_.intersections(&seg);
            if !ints.is_empty() {
                return i - 1;
            }
        }
        start_index
    }

    pub fn trace_to_shape_end(&self, points: &[Point], end_index: usize) -> usize {
        // Simplified: clip to bounding box
        for i in (1..=end_index).rev() {
            let seg = Segment::new(points[i - 1], points[i]);
            let ints = self.box_.intersections(&seg);
            if !ints.is_empty() {
                return i;
            }
        }
        end_index
    }

    /// Is this a sequence diagram container?
    pub fn is_sequence_diagram(&self) -> bool {
        self.shape.value == d2_target::SHAPE_SEQUENCE_DIAGRAM
    }

    /// Is this a sequence diagram group?
    pub fn is_sequence_diagram_group(&self) -> bool {
        false // simplified
    }

    /// Is this a grid diagram?
    pub fn is_grid_diagram(&self) -> bool {
        false // simplified
    }

    /// Get label top-left position (simplified).
    pub fn get_label_top_left(&self) -> Option<Point> {
        let pos_str = self.label_position.as_deref()?;
        let pos = d2_label::Position::from_string(pos_str);
        let b = Box2D::new(self.top_left, self.width, self.height);
        let w = self.label_dimensions.width as f64;
        let h = self.label_dimensions.height as f64;
        Some(pos.get_point_on_box(&b, d2_label::PADDING, w, h))
    }

    /// Get icon top-left position (simplified).
    pub fn get_icon_top_left(&self) -> Option<Point> {
        let pos_str = self.icon_position.as_deref()?;
        let pos = d2_label::Position::from_string(pos_str);
        let b = Box2D::new(self.top_left, self.width, self.height);
        let size = d2_target::MAX_ICON_SIZE as f64;
        Some(pos.get_point_on_box(&b, d2_label::PADDING, size, size))
    }
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Edge {
    pub abs_id: String,

    pub src: ObjId,
    pub dst: ObjId,

    pub src_arrow: bool,
    pub dst_arrow: bool,

    pub src_arrowhead: Option<ArrowheadInfo>,
    pub dst_arrowhead: Option<ArrowheadInfo>,

    pub label: Label,
    pub label_dimensions: Dimensions,
    pub label_position: Option<String>,
    pub label_percentage: Option<f32>,

    pub route: Vec<Point>,
    pub is_curve: bool,

    pub style: Style,
    pub icon_style: IconStyle,

    pub tooltip: Option<ScalarValue>,
    pub link: Option<ScalarValue>,
    pub icon: Option<String>,
    pub icon_position: Option<String>,

    pub language: String,

    pub z_index: i32,
    pub classes: Vec<String>,
}

impl Default for Edge {
    fn default() -> Self {
        Self {
            abs_id: String::new(),
            src: 0,
            dst: 0,
            src_arrow: false,
            dst_arrow: true,
            src_arrowhead: None,
            dst_arrowhead: None,
            label: Label::default(),
            label_dimensions: Dimensions::default(),
            label_position: None,
            label_percentage: None,
            route: Vec::new(),
            is_curve: false,
            style: Style::default(),
            icon_style: IconStyle::default(),
            tooltip: None,
            link: None,
            icon: None,
            icon_position: None,
            language: String::new(),
            z_index: 0,
            classes: Vec::new(),
        }
    }
}

impl Edge {
    /// Absolute ID for export.
    pub fn abs_id(&self) -> &str {
        &self.abs_id
    }

    /// Return text info for this edge (simplified).
    pub fn text(&self) -> MText {
        let font_size = self
            .style
            .font_size
            .as_ref()
            .and_then(|v| v.value.parse().ok())
            .unwrap_or(16);
        MText {
            text: self.label.value.clone(),
            font_size,
            is_bold: self.style.bold.as_ref().is_some_and(|v| v.value == "true"),
            is_italic: self
                .style
                .italic
                .as_ref()
                .is_some_and(|v| v.value == "true"),
            dimensions: self.label_dimensions,
        }
    }

    /// Get edge stroke color based on style and stroke-dash.
    pub fn get_stroke(&self, _stroke_dash: f64) -> &str {
        if let Some(ref s) = self.style.stroke {
            &s.value
        } else {
            d2_color::B1
        }
    }

    /// Trace edge endpoints to the shape boundaries.
    /// Returns (new_start_index, new_end_index).
    pub fn trace_to_shape(
        &self,
        points: &[Point],
        start_index: usize,
        end_index: usize,
        graph: &Graph,
    ) -> (usize, usize) {
        let src = &graph.objects[self.src];
        let dst = &graph.objects[self.dst];

        let new_start = src.trace_to_shape_start(points, start_index);
        let new_end = dst.trace_to_shape_end(points, end_index);
        (new_start, new_end)
    }
}

// ---------------------------------------------------------------------------
// Legend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GraphLegend {
    pub label: String,
    pub objects: Vec<Object>,
    pub edges: Vec<Edge>,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Graph {
    pub name: String,
    pub is_folder_only: bool,

    pub root: ObjId,
    pub objects: Vec<Object>,
    pub edges: Vec<Edge>,

    pub theme: Option<d2_themes::Theme>,
    pub legend: Option<GraphLegend>,

    // Parent graph reference (for nested boards)
    pub parent: Option<Box<Graph>>,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            name: String::new(),
            is_folder_only: false,
            root: 0,
            objects: vec![Object {
                id: "root".to_owned(),
                abs_id: "".to_owned(),
                ..Default::default()
            }],
            edges: Vec::new(),
            theme: None,
            legend: None,
            parent: None,
        }
    }
}

impl Graph {
    /// Create a new empty graph with a root object at index 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the root object.
    pub fn root_obj(&self) -> &Object {
        &self.objects[self.root]
    }

    /// Get a mutable reference to the root object.
    pub fn root_obj_mut(&mut self) -> &mut Object {
        &mut self.objects[self.root]
    }

    /// Add an object to the graph, returning its ObjId.
    pub fn add_object(&mut self, mut obj: Object) -> ObjId {
        let id = self.objects.len();
        if obj.parent.is_none() {
            obj.parent = Some(self.root);
        }
        self.objects.push(obj);

        // Register as child of parent
        let parent_id = self.objects[id].parent.unwrap_or(self.root);
        self.objects[parent_id].children.push(id);
        self.objects[parent_id].children_array.push(id);

        id
    }

    /// Add an edge to the graph, returning its index.
    pub fn add_edge(&mut self, edge: Edge) -> usize {
        let idx = self.edges.len();
        self.edges.push(edge);
        idx
    }

    /// Navigate to a nested board by name.
    pub fn get_board(&self, _name: &str) -> Option<&Graph> {
        // Simplified: no nested boards
        None
    }

    /// Ensure a child object exists at the given path (relative to `parent`).
    /// Creates intermediate objects as needed. Returns the ObjId.
    pub fn ensure_child_of(&mut self, parent: ObjId, ida: &[String]) -> ObjId {
        let mut cur = parent;
        for name in ida {
            // Look for existing child
            let existing = self.objects[cur]
                .children_array
                .iter()
                .find(|&&cid| self.objects[cid].id_val() == name)
                .copied();
            if let Some(cid) = existing {
                cur = cid;
            } else {
                let parent_abs = self.objects[cur].abs_id.clone();
                let abs = if parent_abs.is_empty() {
                    name.clone()
                } else {
                    format!("{}.{}", parent_abs, name)
                };
                let idx = self.objects.len();
                let obj = Object {
                    id: name.clone(),
                    abs_id: abs,
                    label: Label {
                        value: name.clone(),
                        ..Default::default()
                    },
                    parent: Some(cur),
                    ..Default::default()
                };
                self.objects.push(obj);
                self.objects[cur].children.push(idx);
                self.objects[cur].children_array.push(idx);
                cur = idx;
            }
        }
        cur
    }

    /// Ensure a child object exists at the given path from root.
    pub fn ensure_child(&mut self, ida: &[String]) -> ObjId {
        self.ensure_child_of(self.root, ida)
    }

    /// Connect two objects by creating an edge. Returns the edge index.
    pub fn connect(
        &mut self,
        parent: ObjId,
        src_path: &[String],
        dst_path: &[String],
        src_arrow: bool,
        dst_arrow: bool,
        label: &str,
    ) -> Result<usize, String> {
        let src = self.ensure_child_of(parent, src_path);
        let dst = self.ensure_child_of(parent, dst_path);

        let src_id = &self.objects[src].abs_id;
        let dst_id = &self.objects[dst].abs_id;
        let arrow_str = if src_arrow && dst_arrow {
            "<->"
        } else if src_arrow {
            "<-"
        } else if dst_arrow {
            "->"
        } else {
            "--"
        };
        let index = self.edges.len();
        let abs_id = format!("({} {} {})[{}]", src_id, arrow_str, dst_id, index);

        let edge = Edge {
            abs_id,
            src,
            dst,
            src_arrow,
            dst_arrow,
            label: Label {
                value: label.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        self.edges.push(edge);
        Ok(index)
    }

    /// Check if there's an object at the given id_val path from root.
    pub fn has_child(&self, path: &[String]) -> Option<ObjId> {
        let mut cur = self.root;
        for name in path {
            let found = self.objects[cur]
                .children_array
                .iter()
                .find(|&&cid| self.objects[cid].id_val().eq_ignore_ascii_case(name));
            match found {
                Some(&cid) => cur = cid,
                None => return None,
            }
        }
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_creation() {
        let g = Graph::new();
        assert_eq!(g.objects.len(), 1); // root only
        assert_eq!(g.edges.len(), 0);
        assert_eq!(g.root, 0);
    }

    #[test]
    fn add_object_and_edge() {
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
        let _e = g.add_edge(Edge {
            abs_id: "(a -> b)[0]".into(),
            src: a,
            dst: b,
            ..Default::default()
        });
        assert_eq!(g.objects.len(), 3); // root + a + b
        assert_eq!(g.edges.len(), 1);
        assert!(g.objects[a].parent == Some(0));
    }

    #[test]
    fn object_is_container() {
        let mut g = Graph::new();
        let parent = g.add_object(Object {
            id: "parent".into(),
            abs_id: "parent".into(),
            ..Default::default()
        });
        let _child = g.add_object(Object {
            id: "child".into(),
            abs_id: "parent.child".into(),
            parent: Some(parent),
            ..Default::default()
        });
        assert!(g.objects[parent].is_container());
    }
}
