use std::fs;
use std::path::Path;

pub fn read_file(path: &str) -> String {
    fs::read_to_string(Path::new(path)).expect("Failed to read file")
}
