//! d2-latex: LaTeX to SVG rendering using MathJax via Node.js subprocess.
//!
//! Embeds the MathJax bundle and runs it through `node` to produce SVG markup
//! identical to Go's `d2latex.Render` / `d2latex.Measure`.

use std::io::Write;
use std::process::{Command, Stdio};

/// Pixels per ex unit (matches Go d2latex.pxPerEx = 8).
const PX_PER_EX: i32 = 8;

/// Embedded JS sources (same files as Go d2).
static POLYFILLS_JS: &str = include_str!("../polyfills.js");
static MATHJAX_JS: &str = include_str!("../mathjax.js");
static SETUP_JS: &str = include_str!("../setup.js");

/// Double backslashes in the LaTeX string (mirrors Go doubleBackslashes).
fn double_backslashes(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c == '\\' {
            result.push_str("\\\\");
        } else {
            result.push(c);
        }
    }
    result
}

/// Escape a string for embedding in a JS template literal (backtick string).
/// Note: backslashes are NOT escaped here because `double_backslashes` already
/// doubled them for MathJax. We only escape template literal metacharacters.
fn escape_template_literal(s: &str) -> String {
    s.replace('`', "\\`").replace('$', "\\$")
}

/// Render a LaTeX string to SVG markup using MathJax (via Node.js subprocess).
///
/// Returns the SVG string (e.g. `<svg ...>...</svg>`).
pub fn render(latex: &str) -> Result<String, String> {
    let doubled = double_backslashes(latex);
    let escaped = escape_template_literal(&doubled);

    let tail = format!(
        r#"
const result = adaptor.innerHTML(html.convert(`{}`, {{
  em: {},
  ex: {},
}}));
process.stdout.write(result);
"#,
        escaped,
        PX_PER_EX * 2,
        PX_PER_EX
    );

    // Build the full script. Write via stdin to avoid arg-length limits.
    let mut script = String::with_capacity(
        POLYFILLS_JS.len() + MATHJAX_JS.len() + SETUP_JS.len() + tail.len() + 4,
    );
    script.push_str(POLYFILLS_JS);
    script.push('\n');
    script.push_str(MATHJAX_JS);
    script.push('\n');
    script.push_str(SETUP_JS);
    script.push('\n');
    script.push_str(&tail);

    run_node_stdin(&script)
}

/// Measure a LaTeX string, returning (width, height) in pixels.
///
/// Mirrors Go `d2latex.Measure`: renders the LaTeX, parses the SVG dimensions
/// (in ex units), and converts to pixels.
pub fn measure(latex: &str) -> Result<(i32, i32), String> {
    let svg = render(latex)?;

    // Parse width/height from the SVG: width="Xex" height="Yex"
    let w_ex = extract_ex_dim(&svg, "width")?;
    let h_ex = extract_ex_dim(&svg, "height")?;

    let w = (w_ex * PX_PER_EX as f64).ceil() as i32;
    let h = (h_ex * PX_PER_EX as f64).ceil() as i32;
    Ok((w, h))
}

/// Extract a dimension in `ex` units from an SVG tag attribute.
fn extract_ex_dim(svg: &str, attr: &str) -> Result<f64, String> {
    let pattern = format!(r#"{}=""#, attr);
    let start = svg
        .find(&pattern)
        .ok_or_else(|| format!("missing {} in SVG", attr))?
        + pattern.len();
    let end = svg[start..]
        .find("ex\"")
        .ok_or_else(|| format!("missing ex unit for {} in SVG", attr))?
        + start;
    svg[start..end]
        .parse::<f64>()
        .map_err(|e| format!("failed to parse {}: {}", attr, e))
}

/// Run a Node.js script (provided via stdin) and return stdout.
fn run_node_stdin(script: &str) -> Result<String, String> {
    let mut child = Command::new("node")
        .arg("--max-old-space-size=256")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn node: {}", e))?;

    // Write script to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("failed to write to node stdin: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("node failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("node error: {}", stderr));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("invalid UTF-8 from node: {}", e))
}
