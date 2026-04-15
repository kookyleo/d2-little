//! Spike: verify rquickjs + rough.js produces byte-identical JSON vs Goja.
//!
//! Embeds the same rough.js / setup.js that Go uses, evaluates a rectangle
//! primitive, and prints the stringified children so the caller can diff
//! against /tmp/go_out.txt.

use rquickjs::{Context, Runtime};

const ROUGH_JS: &str = include_str!("../../../../d2/d2renderers/d2sketch/rough.js");
const SETUP_JS: &str = include_str!("../../../../d2/d2renderers/d2sketch/setup.js");

fn main() {
    let rt = Runtime::new().expect("create runtime");
    let ctx = Context::full(&rt).expect("create context");

    ctx.with(|ctx| {
        // Load rough.js then setup.js (sets up rc, node, etc.)
        let _: () = ctx
            .eval(ROUGH_JS)
            .unwrap_or_else(|e| panic!("rough.js eval failed: {e:?}"));
        let _: () = ctx
            .eval(SETUP_JS)
            .unwrap_or_else(|e| panic!("setup.js eval failed: {e:?}"));

        // Primitive selection lets the caller spot-check rectangle / ellipse / path.
        // Known divergence: ellipse differs by ~1 ULP vs Goja (Math.sin/cos are
        // implementation-defined in ECMAScript). Go d2sketch truncates the resulting
        // path `d` floats to 6 decimals, so the drift is invisible end-to-end.
        let primitive = std::env::args().nth(1).unwrap_or_else(|| "rect".into());
        let opts = r##"{fill: "#000", stroke: "#000", strokeWidth: 2, fillWeight: 2.0, hachureGap: 16, fillStyle: "solid", bowing: 2, seed: 1}"##;
        let snippet = match primitive.as_str() {
            "rect" => format!("node = rc.rectangle(0, 0, 100, 50, {opts});"),
            "ellipse" => format!("node = rc.ellipse(50, 25, 100, 50, {opts});"),
            "path" => format!(r#"node = rc.path("M 0 0 L 100 50", {opts});"#),
            other => panic!("unknown primitive {other}"),
        };
        let _: () = ctx
            .eval(snippet)
            .unwrap_or_else(|e| panic!("primitive eval failed: {e:?}"));

        let s: String = ctx
            .eval("JSON.stringify(node.children, null, '  ')")
            .unwrap_or_else(|e| panic!("stringify failed: {e:?}"));

        print!("{s}");
    });
}
