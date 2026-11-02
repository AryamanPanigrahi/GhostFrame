//! Build script: embeds every file under game/assets/ into the game binary.
//!
//! Cargo runs this before compiling the game. It walks the assets folder and
//! writes `assets.rs` into Cargo's OUT_DIR: one `include_bytes!` per file,
//! keyed by its path relative to assets/ ("fighter/p1/idle/00.png").
//! render/assets.rs pulls that file in with `include!`.
//!
//! Why embed instead of loading files at runtime: the desktop exe and the
//! browser build then need no assets folder next to them, and a file dropped
//! into game/assets/ (with any number of frames) is picked up on the next
//! build without touching code.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    // Re-run when anything in assets/ is added, removed or changed.
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut files = Vec::new();
    if assets.is_dir() {
        collect(&assets, &mut files);
    }
    // Sorted, so frame 00 comes before 01 and the output is stable.
    files.sort();

    let mut code = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path
            .strip_prefix(&assets)
            .expect("file is under assets/")
            .to_string_lossy()
            .replace('\\', "/");
        let full = path.to_string_lossy().replace('\\', "/");
        code.push_str(&format!("    ({name:?}, include_bytes!({full:?})),\n"));
    }
    code.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out.join("assets.rs"), code).expect("write assets.rs");
}

/// Every .png and .ttf below `dir`. Other files (CREDITS.md, notes) are
/// skipped so they don't bloat the binary.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read assets dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            println!("cargo:rerun-if-changed={}", path.display());
            collect(&path, files);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("png") | Some("ttf")
        ) {
            files.push(path);
        }
    }
}
