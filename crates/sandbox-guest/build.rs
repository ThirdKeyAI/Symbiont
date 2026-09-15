use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn main() {
    let mut hash = Sha256::new();
    fn visit(path: &Path, hash: &mut Sha256) {
        if path.is_dir() {
            let mut entries: Vec<_> = fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                visit(&path, hash);
            }
        } else {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update(fs::read(path).unwrap());
        }
    }
    for path in ["src", "Cargo.toml", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
        visit(Path::new(path), &mut hash);
    }
    println!(
        "cargo:rustc-env=SYMBI_GUEST_IMPLEMENTATION={:x}",
        hash.finalize()
    );
}
