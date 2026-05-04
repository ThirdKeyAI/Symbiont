use std::path::PathBuf;

fn main() {
    let dir: PathBuf = ["tree-sitter-symbiont", "src"].iter().collect();

    let parser = dir.join("parser.c");
    println!("cargo:rerun-if-changed={}", parser.display());

    cc::Build::new()
        .include(&dir)
        .file(parser)
        .compile("tree-sitter-symbiont");
}
