//! Dump the first byte difference for an arbitrary e2e case.
//!
//! Usage: cargo run --example dump_case -- <category> <name> <script>
//! e.g.   cargo run --example dump_case -- stable bold-mono "$(cat - << 'D2'
//!        not bold mono.style.font: mono
//!        not bold mono.style.bold: false
//!        bold mono.style.font: mono
//!        D2
//!        )"

use std::fs;

fn main() {
    let mut args = std::env::args().skip(1);
    let category = args.next().expect("category");
    let name = args.next().expect("name");
    let script = args.next().expect("script");

    let svg = match d2_lib::d2_to_svg(&script) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERR: {}", e);
            std::process::exit(1);
        }
    };
    let svg_str = String::from_utf8_lossy(&svg).to_string();
    let exp_path = format!(
        "crates/d2-lib/tests/e2e_testdata/{}/{}/dagre/sketch.exp.svg",
        category, name
    );
    let exp = fs::read_to_string(&exp_path).expect("read fixture");

    let pos = svg_str
        .chars()
        .zip(exp.chars())
        .position(|(a, b)| a != b)
        .unwrap_or(svg_str.len().min(exp.len()));
    println!("ours len={}, exp len={}", svg_str.len(), exp.len());
    println!("first diff at {}", pos);

    let start = pos.saturating_sub(80);
    let end_o = (pos + 120).min(svg_str.len());
    let end_e = (pos + 120).min(exp.len());
    println!("OURS: {:?}", &svg_str[start..end_o]);
    println!("EXP : {:?}", &exp[start..end_e]);
}
