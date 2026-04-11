fn main() {
    let input = std::env::args().nth(1).unwrap_or_else(|| String::new());
    let opts = d2_lib::CompileOptions {
        pad: Some(0),
        ..d2_lib::CompileOptions::default()
    };
    let (diagram, _) = d2_lib::compile(&input, &opts).unwrap();
    let bytes = d2_target::go_json::diagram_bytes(&diagram);
    println!("{}", String::from_utf8_lossy(&bytes));
    println!("hash: {}", diagram.hash_id(None));
}
