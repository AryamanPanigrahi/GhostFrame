//! Textures and the font, built once at startup from the files that
//! build.rs embedded from game/assets/. Render-only: nothing here is read by
//! the sim.
//!
//! Every lookup is optional. A missing file (or a whole missing folder)
//! makes the caller fall back to the placeholder boxes and the default
//! font, so the game always runs, with or without art.

use macroquad::prelude::*;
use macroquad::text::{load_ttf_font_from_bytes, Font};
use std::collections::BTreeMap;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}

pub struct Assets {
    /// Single images by path, e.g. "hud/bar_frame.png".
    images: BTreeMap<String, Texture2D>,
    /// Animations by folder, e.g. "fighter/p1/idle", frames in order.
    anims: BTreeMap<String, Vec<Texture2D>>,
    pub font: Option<Font>,
}

impl Assets {
    /// Decode every embedded file. Needs the window (GL context) to exist,
    /// so call it from inside `main`.
    pub fn load() -> Assets {
        let mut images = BTreeMap::new();
        let mut anims: BTreeMap<String, Vec<Texture2D>> = BTreeMap::new();
        let mut font = None;

        // FILES is sorted by path, so frames arrive as 00, 01, 02...
        for (name, bytes) in embedded::FILES {
            if name.ends_with(".ttf") {
                if *name == "font/main.ttf" {
                    font = load_ttf_font_from_bytes(bytes).ok();
                }
                continue;
            }
            let texture = Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png));
            // Pixel art: scale with hard edges, never blur.
            texture.set_filter(FilterMode::Nearest);
            match name.rsplit_once('/') {
                // "folder/00.png": a numbered frame of an animation.
                Some((folder, file)) if is_frame(file) => {
                    anims.entry(folder.to_owned()).or_default().push(texture);
                }
                _ => {
                    images.insert((*name).to_owned(), texture);
                }
            }
        }
        if let Some(font) = &mut font {
            font.set_filter(FilterMode::Nearest);
        }
        Assets {
            images,
            anims,
            font,
        }
    }

    pub fn image(&self, name: &str) -> Option<&Texture2D> {
        self.images.get(name)
    }

    /// Frames of an animation folder; None if the folder has no frames.
    pub fn anim(&self, folder: &str) -> Option<&[Texture2D]> {
        self.anims
            .get(folder)
            .map(Vec::as_slice)
            .filter(|f| !f.is_empty())
    }
}

/// "00.png", "07.png": a two-digit frame number.
fn is_frame(file: &str) -> bool {
    match file.strip_suffix(".png") {
        Some(stem) => !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_names() {
        assert!(is_frame("00.png"));
        assert!(is_frame("12.png"));
        assert!(!is_frame("bar_fill.png"));
        assert!(!is_frame(".png"));
        assert!(!is_frame("00.ttf"));
    }

    #[test]
    fn embedded_files_have_forward_slash_names() {
        for (name, bytes) in embedded::FILES {
            assert!(!name.contains('\\'), "{name}");
            assert!(!bytes.is_empty(), "{name}");
        }
    }
}
