//! E2E test runner using subprocess isolation for each case.
//! This prevents stack overflow in one case from aborting the entire suite.
//!
//! Run with: cargo test -p d2-lib --test e2e_runner -- --nocapture

use std::process::Command;

fn fixture_path(category: &str, name: &str) -> std::path::PathBuf {
    // The Go d2 e2e harness writes fixtures to directories whose names have
    // spaces replaced with underscores (testdata/<category>/<name>/dagre/...).
    // Keep that translation here so a case named "1 to 2" looks up
    // testdata/sanity/1_to_2/dagre/sketch.exp.svg.
    let dir_name = name.replace(' ', "_");
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e_testdata")
        .join(category)
        .join(dir_name)
        .join("dagre")
        .join("sketch.exp.svg")
}

/// When invoked as a subprocess with E2E_CASE_INDEX=N, run just that case.
fn maybe_run_single_case() -> bool {
    let idx_str = match std::env::var("E2E_CASE_INDEX") {
        Ok(s) => s,
        Err(_) => return false,
    };
    let idx: usize = idx_str.parse().unwrap();
    let cases_json = include_str!("e2e_cases.json");
    let cases: Vec<serde_json::Value> = serde_json::from_str(cases_json).unwrap();
    let case = &cases[idx];
    let script = case["script"].as_str().unwrap();

    match d2_lib::d2_to_svg(script) {
        Ok(svg) => {
            // Write SVG to stdout for parent to capture
            print!("{}", String::from_utf8_lossy(&svg));
        }
        Err(e) => {
            eprint!("ERR:{}", e);
            std::process::exit(1);
        }
    }
    true
}

#[test]
fn e2e_full_dashboard() {
    // If we're a subprocess, run the single case
    if maybe_run_single_case() {
        return;
    }

    let cases_json = include_str!("e2e_cases.json");
    let cases: Vec<serde_json::Value> = serde_json::from_str(cases_json).unwrap();

    let test_binary = std::env::current_exe().unwrap();

    let mut pass = 0usize;
    let mut svg_diff = 0usize;
    let mut compile_err = 0usize;
    let mut timeout = 0usize;
    let mut no_fixture = 0usize;
    let mut failures: Vec<String> = Vec::new();

    // Match Go's `#01`, `#02` fixture suffixes when two cases share a
    // name within a category.
    let mut seen: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();

    for (i, case) in cases.iter().enumerate() {
        let raw_name = case["name"].as_str().unwrap();
        let category = case["category"].as_str().unwrap();
        let key = (category.to_string(), raw_name.to_string());
        let idx = *seen.entry(key).and_modify(|c| *c += 1).or_insert(0);
        let name: std::borrow::Cow<'_, str> = if idx == 0 {
            std::borrow::Cow::Borrowed(raw_name)
        } else {
            std::borrow::Cow::Owned(format!("{}#{:02}", raw_name, idx))
        };
        let name = name.as_ref();

        eprint!("[{}/{}] {} ... ", i + 1, cases.len(), name);

        // Run as subprocess with manual timeout
        let mut child = match Command::new(&test_binary)
            .env("E2E_CASE_INDEX", i.to_string())
            .arg("e2e_full_dashboard")
            .arg("--nocapture")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SPAWN ERR: {}", e);
                compile_err += 1;
                continue;
            }
        };

        // Wait with timeout
        let start = std::time::Instant::now();
        let output = loop {
            match child.try_wait() {
                Ok(Some(_)) => break child.wait_with_output(),
                Ok(None) => {
                    if start.elapsed() > std::time::Duration::from_secs(15) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => break Err(e),
            }
        };

        let svg_bytes = match output {
            Err(_) => {
                eprintln!("TIMEOUT/CRASH");
                timeout += 1;
                failures.push(format!("[{}] {}: TIMEOUT/CRASH", category, name));
                continue;
            }
            Ok(ref out) if !out.status.success() => {
                let err = String::from_utf8_lossy(&out.stderr);
                let msg = if err.contains("overflow") {
                    "STACK OVERFLOW"
                } else {
                    &err[..err.len().min(80)]
                };
                eprintln!("ERR: {}", msg);
                compile_err += 1;
                failures.push(format!("[{}] {}: {}", category, name, msg));
                continue;
            }
            Ok(out) => out.stdout,
        };

        // Strip the libtest harness preamble/postamble that wraps any
        // child stdout when running with `--nocapture`. The child writes a
        // single SVG via `print!` so the bytes between `<?xml` and the last
        // `</svg>` are the payload.
        let svg_str_full = String::from_utf8_lossy(&svg_bytes);
        let svg_str: std::borrow::Cow<'_, str> = if let Some(start) = svg_str_full.find("<?xml") {
            if let Some(end) = svg_str_full.rfind("</svg>") {
                std::borrow::Cow::Owned(svg_str_full[start..end + "</svg>".len()].to_string())
            } else {
                svg_str_full.clone()
            }
        } else {
            svg_str_full.clone()
        };
        if !svg_str.contains("<svg") {
            eprintln!("NO SVG");
            compile_err += 1;
            continue;
        }

        let expected_path = fixture_path(category, name);
        if !expected_path.exists() {
            eprintln!("OK (no fixture)");
            no_fixture += 1;
            continue;
        }

        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
        if *svg_str == expected {
            eprintln!("MATCH");
            pass += 1;
        } else {
            let pos = svg_str
                .chars()
                .zip(expected.chars())
                .position(|(a, b)| a != b)
                .unwrap_or(svg_str.len().min(expected.len()));
            eprintln!("DIFF@{} ({}b vs {}b)", pos, svg_str.len(), expected.len());
            svg_diff += 1;
            failures.push(format!(
                "[{}] {}: DIFF@{} ({}b vs {}b)",
                category,
                name,
                pos,
                svg_str.len(),
                expected.len()
            ));
        }
    }

    println!("\n========================================");
    println!("   E2E Dashboard: {} cases", cases.len());
    println!("========================================");
    println!("  MATCH:     {:>3} (byte-identical SVG)", pass);
    println!("  DIFF:      {:>3} (SVG output differs)", svg_diff);
    println!("  COMPILE:   {:>3} (compilation error)", compile_err);
    println!("  TIMEOUT:   {:>3} (>15s, likely infinite loop)", timeout);
    println!("  NO_FIX:    {:>3} (no expected fixture)", no_fixture);
    println!(
        "  RATE:      {:.1}%",
        pass as f64 / cases.len() as f64 * 100.0
    );
    println!("========================================");

    if !failures.is_empty() {
        println!("\nFailures (first 30):");
        for f in failures.iter().take(30) {
            println!("  {}", f);
        }
    }
}
