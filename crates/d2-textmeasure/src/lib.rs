//! Text measurement for d2 rendering.
//!
//! Ported from Go `lib/textmeasure/textmeasure.go` and `lib/textmeasure/atlas.go`.
//!
//! The Go code uses `golang/freetype/truetype` for font parsing and `fixed.Int26_6`
//! for sub-pixel precision. `rusttype` is a direct Rust port of the same Go freetype
//! library, so the measurements should be byte-identical.

use std::collections::HashMap;

use d2_fonts::{FONT_FAMILIES, FONT_STYLES, Font, FontFamily, FontStyle};
use rusttype::{Font as RtFont, Scale};
use unicode_segmentation::UnicodeSegmentation;

const TAB_SIZE: f64 = 4.0;
const SIZELESS_FONT_SIZE: i32 = 0;

/// The set of runes for which we pre-build glyph atlases.
/// ASCII + Latin-1 Supplement + Geometric Shapes (matches Go init()).
fn default_runes() -> Vec<char> {
    let mut runes = Vec::with_capacity(512);
    // ASCII (U+0000..U+007F)
    for c in 0x0000u32..=0x007F {
        if let Some(ch) = char::from_u32(c) {
            runes.push(ch);
        }
    }
    // Latin-1 Supplement (U+0080..U+00FF)
    for c in 0x0080u32..=0x00FF {
        if let Some(ch) = char::from_u32(c) {
            runes.push(ch);
        }
    }
    // Geometric Shapes (U+25A0..U+25FF)
    for c in 0x25A0u32..=0x25FF {
        if let Some(ch) = char::from_u32(c) {
            runes.push(ch);
        }
    }
    runes
}

// ---------------------------------------------------------------------------
// Rect (internal bounding-box type, mirrors Go rect)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Rect {
    tl_x: f64,
    tl_y: f64,
    br_x: f64,
    br_y: f64,
}

impl Rect {
    fn zero() -> Self {
        Self {
            tl_x: 0.0,
            tl_y: 0.0,
            br_x: 0.0,
            br_y: 0.0,
        }
    }

    fn w(&self) -> f64 {
        self.br_x - self.tl_x
    }

    fn h(&self) -> f64 {
        self.br_y - self.tl_y
    }

    fn norm(self) -> Self {
        Self {
            tl_x: self.tl_x.min(self.br_x),
            tl_y: self.tl_y.min(self.br_y),
            br_x: self.tl_x.max(self.br_x),
            br_y: self.tl_y.max(self.br_y),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            tl_x: self.tl_x.min(other.tl_x),
            tl_y: self.tl_y.min(other.tl_y),
            br_x: self.br_x.max(other.br_x),
            br_y: self.br_y.max(other.br_y),
        }
    }
}

// ---------------------------------------------------------------------------
// Glyph + Atlas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Glyph {
    dot_x: f64,
    dot_y: f64,
    frame: Rect,
    advance: f64,
}

/// Atlas holds pre-computed glyph metrics for a set of runes at a specific
/// font face / size.
struct Atlas {
    mapping: HashMap<char, Glyph>,
    ascent: f64,
    descent: f64,
    line_height: f64,
    /// Precomputed kern pairs. Key = (prev_char, char).
    kern_cache: HashMap<(char, char), f64>,
}

/// Convert a rusttype `fixed.Int26_6`-equivalent value to f64.
/// rusttype uses f32 internally but we promote to f64 to match Go precision.
fn i2f(v: f32) -> f64 {
    v as f64
}

impl Atlas {
    /// Build an atlas from a parsed font at the given pixel size.
    fn new(font: &RtFont<'static>, size: f64, runes: &[char]) -> Self {
        let scale = Scale::uniform(size as f32);
        let v_metrics = font.v_metrics(scale);
        let ascent = i2f(v_metrics.ascent);
        let descent = i2f(-v_metrics.descent); // Go stores descent as positive
        let line_height = i2f(v_metrics.ascent - v_metrics.descent + v_metrics.line_gap);

        let mut mapping = HashMap::new();
        // Always include replacement char
        let replacement = '\u{FFFD}';
        let all_runes: Vec<char> = std::iter::once(replacement)
            .chain(runes.iter().copied())
            .collect();

        // Build the glyph layout similar to Go makeSquareMapping.
        // The critical metrics for measurement are: advance, bounds, and dot position.
        // We replicate the Go atlas layout logic to ensure identical results.
        let padding = 2.0f32; // fixed.I(2) in Go

        // We need to do the full layout to compute bounds, then build mapping.
        // Replicate makeSquareMapping: binary search for optimal width.
        let make_mapping = |width: f32| -> (
            HashMap<char, (f32, f32, f32, f32, f32, f32, f32)>,
            (f32, f32, f32, f32),
        ) {
            // Returns map of char -> (dot_x, dot_y, frame_min_x, frame_min_y, frame_max_x, frame_max_y, advance)
            // and overall bounds (min_x, min_y, max_x, max_y)
            let mut result = HashMap::new();
            let mut bounds = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut dx: f32 = 0.0;
            let mut dy: f32 = 0.0;

            for &r in &all_runes {
                let glyph = font.glyph(r);
                let scaled = glyph.scaled(scale);
                let h_metrics = scaled.h_metrics();
                let advance = h_metrics.advance_width;

                // Get glyph bounds
                let glyph_bounds = scaled.exact_bounding_box();
                let (b_min_x, b_min_y, b_max_x, b_max_y) = match glyph_bounds {
                    Some(bb) => (bb.min.x, bb.min.y, bb.max.x, bb.max.y),
                    None => (0.0, 0.0, 0.0, 0.0),
                };

                // Floor/ceil like Go code
                let frame_min_x = b_min_x.floor();
                let frame_min_y = b_min_y.floor();
                let frame_max_x = b_max_x.ceil();
                let frame_max_y = b_max_y.ceil();

                // Shift dot so frame starts at current position
                dx -= frame_min_x;
                let shifted_frame_min_x = frame_min_x + dx;
                let shifted_frame_min_y = frame_min_y + dy;
                let shifted_frame_max_x = frame_max_x + dx;
                let shifted_frame_max_y = frame_max_y + dy;

                result.insert(
                    r,
                    (
                        dx,
                        dy,
                        shifted_frame_min_x,
                        shifted_frame_min_y,
                        shifted_frame_max_x,
                        shifted_frame_max_y,
                        advance,
                    ),
                );

                // Update bounds
                bounds.0 = bounds.0.min(shifted_frame_min_x);
                bounds.1 = bounds.1.min(shifted_frame_min_y);
                bounds.2 = bounds.2.max(shifted_frame_max_x);
                bounds.3 = bounds.3.max(shifted_frame_max_y);

                dx = shifted_frame_max_x;
                // padding + align to integer
                dx += padding;
                dx = dx.ceil();

                // Width exceeded? New row.
                if shifted_frame_max_x >= width {
                    dx = 0.0;
                    dy += v_metrics.ascent + (-v_metrics.descent);
                    dy += padding;
                    dy = dy.ceil();
                }
            }

            (result, bounds)
        };

        // Binary search for square-ish mapping (matches Go makeSquareMapping)
        let max_width = 1024.0 * 1024.0;
        let mut lo = 0i32;
        let mut hi = (max_width * 64.0) as i32; // Int26_6 scale
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let w = mid as f32 / 64.0;
            let (_, bounds) = make_mapping(w);
            let bw = bounds.2 - bounds.0;
            let bh = bounds.3 - bounds.1;
            if bw >= bh {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let best_width = lo as f32 / 64.0;
        let (raw_mapping, bounds) = make_mapping(best_width);

        let _bounds_tl_x = i2f(bounds.0);
        let bounds_tl_y = i2f(bounds.1);
        let _bounds_br_x = i2f(bounds.2);
        let bounds_br_y = i2f(bounds.3);

        for (&r, &(dx, dy, fmin_x, fmin_y, fmax_x, fmax_y, adv)) in &raw_mapping {
            let dot_x_f = i2f(dx);
            let dot_y_f = bounds_br_y - (i2f(dy) - bounds_tl_y);

            let frame_tl_x = i2f(fmin_x);
            let frame_tl_y = bounds_br_y - (i2f(fmin_y) - bounds_tl_y);
            let frame_br_x = i2f(fmax_x);
            let frame_br_y = bounds_br_y - (i2f(fmax_y) - bounds_tl_y);

            let frame = Rect {
                tl_x: frame_tl_x,
                tl_y: frame_tl_y,
                br_x: frame_br_x,
                br_y: frame_br_y,
            }
            .norm();

            mapping.insert(
                r,
                Glyph {
                    dot_x: dot_x_f,
                    dot_y: dot_y_f,
                    frame,
                    advance: i2f(adv),
                },
            );
        }

        // Precompute kern pairs
        let mut kern_cache = HashMap::new();
        for &r0 in &all_runes {
            for &r1 in &all_runes {
                let kern = font.pair_kerning(scale, font.glyph(r0).id(), font.glyph(r1).id());
                if kern != 0.0 {
                    kern_cache.insert((r0, r1), i2f(kern));
                }
            }
        }

        Self {
            mapping,
            ascent,
            descent,
            line_height,
            kern_cache,
        }
    }

    fn contains(&self, r: char) -> bool {
        self.mapping.contains_key(&r)
    }

    fn glyph(&self, r: char) -> Glyph {
        self.mapping
            .get(&r)
            .copied()
            .unwrap_or(self.mapping[&'\u{FFFD}'])
    }

    fn kern(&self, r0: char, r1: char) -> f64 {
        self.kern_cache.get(&(r0, r1)).copied().unwrap_or(0.0)
    }

    /// Draw a rune and return (rect, frame, bounds, new_dot).
    fn draw_rune(
        &self,
        prev_r: Option<char>,
        r: char,
        dot_x: f64,
        dot_y: f64,
    ) -> (Rect, Rect, Rect, f64, f64) {
        let r = if self.contains(r) { r } else { '\u{FFFD}' };
        if !self.contains('\u{FFFD}') {
            return (Rect::zero(), Rect::zero(), Rect::zero(), dot_x, dot_y);
        }

        let mut dx = dot_x;
        let dy = dot_y;

        let prev = prev_r.unwrap_or('\u{FFFD}');
        if prev_r.is_some() {
            let prev_effective = if self.contains(prev) {
                prev
            } else {
                '\u{FFFD}'
            };
            dx += self.kern(prev_effective, r);
        }

        let glyph = self.glyph(r);

        let sub_x = dx - glyph.dot_x;
        let sub_y = dy - glyph.dot_y;

        let rect2 = Rect {
            tl_x: glyph.frame.tl_x + sub_x,
            tl_y: glyph.frame.tl_y + sub_y,
            br_x: glyph.frame.br_x + sub_x,
            br_y: glyph.frame.br_y + sub_y,
        };

        let mut bounds = rect2;
        if bounds.w() * bounds.h() != 0.0 {
            bounds = Rect {
                tl_x: bounds.tl_x,
                tl_y: dy - self.descent,
                br_x: bounds.br_x,
                br_y: dy + self.ascent,
            };
        }

        let new_dx = dx + glyph.advance;
        (rect2, glyph.frame, bounds, new_dx, dy)
    }
}

// ---------------------------------------------------------------------------
// Ruler
// ---------------------------------------------------------------------------

/// Text measurement ruler - holds font atlases for each font/size combination.
pub struct Ruler {
    /// Origin point
    orig_x: f64,
    orig_y: f64,
    /// Current dot position
    dot_x: f64,
    dot_y: f64,
    /// Line height factor (default 1.0)
    pub line_height_factor: f64,

    line_heights: HashMap<FontKey, f64>,
    tab_widths: HashMap<FontKey, f64>,
    atlases: HashMap<FontKey, Atlas>,
    /// Parsed TTF fonts (keyed by family+style, size-agnostic)
    ttfs: HashMap<FontKey, RtFont<'static>>,

    prev_r: Option<char>,
    bounds: Rect,
    bounds_with_dot: bool,
}

/// Key for atlas / line_height / tab_width lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    family: FontFamily,
    style: FontStyle,
    size: i32,
}

impl From<Font> for FontKey {
    fn from(f: Font) -> Self {
        Self {
            family: f.family,
            style: f.style,
            size: f.size,
        }
    }
}

impl FontKey {
    fn sizeless(self) -> Self {
        Self {
            size: SIZELESS_FONT_SIZE,
            ..self
        }
    }
}

impl Ruler {
    /// Create a new Ruler with all built-in font faces loaded.
    pub fn new() -> Result<Self, String> {
        let mut ttfs = HashMap::new();

        for &family in FONT_FAMILIES {
            for &style in FONT_STYLES {
                let key = FontKey {
                    family,
                    style,
                    size: SIZELESS_FONT_SIZE,
                };
                if ttfs.contains_key(&key) {
                    continue;
                }
                let face_data = d2_fonts::lookup_font_face(family, style);
                let font = RtFont::try_from_bytes(face_data)
                    .ok_or_else(|| format!("failed to parse font {:?} {:?}", family, style))?;
                ttfs.insert(key, font);
            }
        }

        Ok(Self {
            orig_x: 0.0,
            orig_y: 0.0,
            dot_x: 0.0,
            dot_y: 0.0,
            line_height_factor: 1.0,
            line_heights: HashMap::new(),
            tab_widths: HashMap::new(),
            atlases: HashMap::new(),
            ttfs,
            prev_r: None,
            bounds: Rect::zero(),
            bounds_with_dot: false,
        })
    }

    fn add_font_size(&mut self, font: Font) {
        let key = FontKey::from(font);
        let sizeless = key.sizeless();
        let runes = default_runes();

        // Clone the font data reference for the atlas
        let rt_font = self.ttfs[&sizeless].clone();
        let atlas = Atlas::new(&rt_font, font.size as f64, &runes);

        let lh = atlas.line_height;
        let tw = atlas.glyph(' ').advance * TAB_SIZE;

        self.line_heights.insert(key, lh);
        self.tab_widths.insert(key, tw);
        self.atlases.insert(key, atlas);
    }

    fn clear(&mut self) {
        self.prev_r = None;
        self.bounds = Rect::zero();
        self.dot_x = self.orig_x;
        self.dot_y = self.orig_y;
    }

    fn control_rune(&self, r: char, dot_x: f64, dot_y: f64, font: Font) -> Option<(f64, f64)> {
        let key = FontKey::from(font);
        match r {
            '\n' => {
                let new_x = self.orig_x;
                let new_y = dot_y - self.line_height_factor * self.line_heights[&key];
                Some((new_x, new_y))
            }
            '\r' => Some((self.orig_x, dot_y)),
            '\t' => {
                let tw = self.tab_widths[&key];
                let mut rem = (dot_x - self.orig_x) % tw;
                rem = rem % (rem + tw);
                if rem == 0.0 {
                    rem = tw;
                }
                Some((dot_x + rem, dot_y))
            }
            _ => None,
        }
    }

    fn draw_buf(&mut self, font: Font, text: &str) {
        let key = FontKey::from(font);
        for ch in text.chars() {
            if let Some((nx, ny)) = self.control_rune(ch, self.dot_x, self.dot_y, font) {
                self.dot_x = nx;
                self.dot_y = ny;
                continue;
            }

            let (_, _, bounds, new_dx, new_dy) =
                self.atlases[&key].draw_rune(self.prev_r, ch, self.dot_x, self.dot_y);

            self.prev_r = Some(ch);
            self.dot_x = new_dx;
            self.dot_y = new_dy;

            if self.bounds_with_dot {
                let dot_rect = Rect {
                    tl_x: self.dot_x,
                    tl_y: self.dot_y,
                    br_x: self.dot_x,
                    br_y: self.dot_y,
                };
                self.bounds = self.bounds.union(dot_rect);
                self.bounds = self.bounds.union(bounds);
            } else {
                if self.bounds.w() * self.bounds.h() == 0.0 {
                    self.bounds = bounds;
                } else {
                    self.bounds = self.bounds.union(bounds);
                }
            }
        }
    }

    /// Measure text precisely (returns exact floating-point width and height).
    pub fn measure_precise(&mut self, font: Font, s: &str) -> (f64, f64) {
        let key = FontKey::from(font);
        if !self.atlases.contains_key(&key) {
            self.add_font_size(font);
        }
        self.clear();
        self.draw_buf(font, s);
        (self.bounds.w(), self.bounds.h())
    }

    /// Measure text (returns ceiled integer width and height).
    /// Also applies Unicode grapheme-cluster scaling.
    pub fn measure(&mut self, font: Font, s: &str) -> (i32, i32) {
        let (w, h) = self.measure_precise(font, s);
        let w = self.scale_unicode(w, font, s);
        (w.ceil() as i32, h.ceil() as i32)
    }

    /// Measure monospace text (includes dot in bounds).
    pub fn measure_mono(&mut self, font: Font, s: &str) -> (i32, i32) {
        let orig = self.bounds_with_dot;
        self.bounds_with_dot = true;
        let result = self.measure(font, s);
        self.bounds_with_dot = orig;
        result
    }

    fn space_width(&mut self, font: Font) -> f64 {
        let key = FontKey::from(font);
        if !self.atlases.contains_key(&key) {
            self.add_font_size(font);
        }
        self.atlases[&key].glyph(' ').advance
    }

    fn scale_unicode(&mut self, mut w: f64, font: Font, s: &str) -> f64 {
        // Check if grapheme cluster count differs from byte length
        // (indicates multi-codepoint graphemes like emoji)
        let grapheme_count = s.graphemes(true).count();
        if grapheme_count != s.len() {
            for line in s.split('\n') {
                let (line_w, _) = self.measure_precise(font, line);
                let mut adjusted_w = line_w;

                let mono = Font::new(FontFamily::SourceCodePro, font.style, font.size);
                for grapheme in line.graphemes(true) {
                    // Skip single-character graphemes
                    if grapheme.chars().count() == 1 {
                        continue;
                    }

                    // Measure the grapheme using the current font
                    let key = FontKey::from(font);
                    let mut prev_r: Option<char> = None;
                    let dot_x_start = self.orig_x;
                    let dot_y_start = self.orig_y;
                    let mut dx = dot_x_start;
                    let mut dy = dot_y_start;
                    let mut b = Rect::zero();

                    for ch in grapheme.chars() {
                        if let Some((nx, ny)) = self.control_rune(ch, dx, dy, font) {
                            dx = nx;
                            dy = ny;
                            continue;
                        }
                        let (_, _, bounds, new_dx, new_dy) =
                            self.atlases[&key].draw_rune(prev_r, ch, dx, dy);
                        b = b.union(bounds);
                        prev_r = Some(ch);
                        dx = new_dx;
                        dy = new_dy;
                    }

                    adjusted_w -= b.w();
                    // Use grapheme width (unicode width) * monospace space width
                    let unicode_width =
                        unicode_segmentation::UnicodeSegmentation::graphemes(grapheme, true)
                            .count();
                    adjusted_w += self.space_width(mono) * unicode_width as f64;
                }

                w = w.max(adjusted_w);
            }
        }
        w
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use d2_fonts::*;

    #[test]
    fn test_ruler_creation() {
        let ruler = Ruler::new();
        assert!(ruler.is_ok());
    }

    #[test]
    fn test_measure_precise_basic() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);
        let (w, h) = ruler.measure_precise(font, "Hello");
        assert!(w > 0.0, "width should be positive, got {}", w);
        assert!(h > 0.0, "height should be positive, got {}", h);
    }

    #[test]
    fn test_measure_hello_regular_16() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);
        let (w, h) = ruler.measure_precise(font, "Hello");
        // Verify reasonable range (Go produces around 30-35 width, 20-24 height for this)
        assert!(w > 20.0 && w < 50.0, "unexpected width: {}", w);
        assert!(h > 10.0 && h < 30.0, "unexpected height: {}", h);
    }

    #[test]
    fn test_measure_increasing_chars() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);
        let text = "abcdefghij";
        for i in 1..text.len() {
            let (w1, h1) = ruler.measure(font, &text[..i]);
            let (w2, h2) = ruler.measure(font, &text[..i + 1]);
            assert_eq!(h1, h2, "height should not change for single line");
            assert!(
                w1 < w2,
                "width should increase: '{}' ({}) vs '{}' ({})",
                &text[..i],
                w1,
                &text[..i + 1],
                w2
            );
        }
    }

    #[test]
    fn test_measure_newlines_increase_height() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);
        let (_, h1) = ruler.measure(font, "Hello");
        let (_, h2) = ruler.measure(font, "Hello\nWorld");
        assert!(h2 > h1, "newline should increase height: {} vs {}", h1, h2);
    }

    #[test]
    fn test_font_sizes_increasing() {
        let mut ruler = Ruler::new().unwrap();
        let text = "The quick brown fox";
        for i in 0..FONT_SIZES.len() - 1 {
            let f1 = FontFamily::SourceSansPro.font(FONT_SIZES[i], FontStyle::Regular);
            let f2 = FontFamily::SourceSansPro.font(FONT_SIZES[i + 1], FontStyle::Regular);
            let (w1, h1) = ruler.measure(f1, text);
            let (w2, h2) = ruler.measure(f2, text);
            assert!(
                w1 < w2,
                "larger font size should produce wider text: size {} ({}) vs size {} ({})",
                FONT_SIZES[i],
                w1,
                FONT_SIZES[i + 1],
                w2
            );
            assert!(
                h1 < h2,
                "larger font size should produce taller text: size {} ({}) vs size {} ({})",
                FONT_SIZES[i],
                h1,
                FONT_SIZES[i + 1],
                h2
            );
        }
    }

    #[test]
    fn test_measure_empty_string() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);
        let (w, h) = ruler.measure(font, "");
        assert_eq!(w, 0);
        assert_eq!(h, 0);
    }

    #[test]
    fn test_measure_single_chars() {
        let mut ruler = Ruler::new().unwrap();
        let font = FontFamily::SourceSansPro.font(FONT_SIZE_M, FontStyle::Regular);

        // "a" should have width ~9 at size 16
        let (w, h) = ruler.measure(font, "a");
        assert!(w > 0, "single 'a' width should be > 0, got {}", w);
        assert!(h > 0, "single 'a' height should be > 0, got {}", h);

        // "w" should be wider than "a" (typically)
        let (wa, _) = ruler.measure(font, "a");
        let (ww, _) = ruler.measure(font, "w");
        assert!(
            ww >= wa,
            "'w' should be at least as wide as 'a': {} vs {}",
            ww,
            wa
        );
    }
}
