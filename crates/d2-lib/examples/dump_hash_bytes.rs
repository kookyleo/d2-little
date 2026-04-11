//! Dump the diagram_bytes (the FNV-1a input) for a given script so we can
//! compare against Go's `Diagram.Bytes()` output and find what part of the
//! JSON serialization differs.
//!
//! Usage: cargo run --example dump_hash_bytes -- "<script>"

use std::io::Write;

fn main() {
    let script = std::env::args().nth(1).expect("script");
    let opts = d2_lib::CompileOptions {
        pad: Some(0),
        ..d2_lib::CompileOptions::default()
    };
    let (diagram, _svg) = d2_lib::compile(&script, &opts).expect("compile");

    let bytes = d2_target::go_json::diagram_bytes(&diagram);
    let h = diagram.hash_id(None);
    eprintln!("len={} hash={}", bytes.len(), h);
    std::io::stdout().write_all(&bytes).unwrap();
}
