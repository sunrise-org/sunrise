//! OptiFine custom lightmaps (`sunrise_optifine::lightmap`) on the engine's
//! [`crate::custom_lightmap::Lightmaps`]: a pack's pictures, decoded once with the pack, and
//! each frame's texture from them. A pack without `optifine/lightmap` has none.

use crate::custom_lightmap::{Input, Lightmaps, Pixels};
use crate::resource_source::ResourceSource;
use sunrise_optifine::lightmap::{self as optifine, Files, Picture};

/// A pack's decoded lightmaps.
pub struct CustomLightmaps(optifine::Lightmaps);

/// The widest picture read: 4096 columns of 64 rows (a lightmap is a few dozen columns).
const MAX_WIDTH: u32 = 4096;
/// The most bytes read of one file, before it is decoded.
const MAX_BYTES: u64 = 4 << 20;

/// A PNG's width and height, from the header's first 24 bytes; none for anything else.
fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if head.len() < 24 || &head[..8] != SIGNATURE || &head[12..16] != b"IHDR" {
        return None;
    }
    let number =
        |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    Some((number(16), number(20)))
}

/// The picture at `path` as width, height and RGBA: one read of at most [`MAX_BYTES`], whose own
/// PNG header must give a lightmap's size within the limits before it is decoded; none with a
/// warning otherwise.
fn decode(root: &ResourceSource, path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = match root.read_limited(path, MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("{path}: {e}");
            return None;
        }
    };
    let Some((width, height)) = png_size(&bytes) else {
        log::warn!("{path}: a lightmap is a PNG");
        return None;
    };
    if !Picture::valid_size(width, height) || width > MAX_WIDTH {
        log::warn!("{path}: a lightmap is 1 to {MAX_WIDTH} pixels wide and 32 or 64 tall");
        return None;
    }
    match crate::hud::decode_image_bytes(&bytes) {
        Ok(image) if (image.0, image.1) == (width, height) => Some(image),
        Ok(_) => {
            log::warn!("{path}: the picture's size is not its header's");
            None
        }
        Err(e) => {
            log::warn!("{path}: {e:#}");
            None
        }
    }
}

impl CustomLightmaps {
    /// The lightmaps of the overlay at `root`, for the worlds that have one
    /// (`sunrise_optifine::lightmap::WORLDS`). A picture that does not decode is dropped with a
    /// warning.
    pub fn load(root: &ResourceSource) -> Self {
        let files = Files::scan(root);
        if files.is_empty() {
            return Self(Default::default());
        }
        Self(optifine::Lightmaps::load(&files, |path| decode(root, path)))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Lightmaps for CustomLightmaps {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn has(&self, world: i32) -> bool {
        self.0.has(world)
    }

    fn pixels(&self, world: i32, input: &Input) -> Option<Pixels> {
        self.0.pixels(
            world,
            &optifine::Input {
                sky: input.sky,
                lightning: input.weather.lightning,
                flicker: input.flicker,
                gamma: input.gamma,
                night_vision: input.night_vision,
                darkness: input.darkness,
                rain: input.weather.rain,
                thunder: input.weather.thunder,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32) -> Vec<u8> {
        let mut head = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        head.extend(width.to_be_bytes());
        head.extend(height.to_be_bytes());
        head
    }

    #[test]
    fn the_header_gives_the_size_before_decoding() {
        assert_eq!(png_size(&header(16, 32)), Some((16, 32)));
        assert_eq!(png_size(&header(16, 32)[..20]), None);
        assert_eq!(png_size(b"GIF89a.................."), None);
    }

    /// A whole, decodable PNG of `width` by `height`.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![255; (width * height * 4) as usize])
            .unwrap();
        drop(writer);
        out
    }

    #[test]
    fn whole_pngs_of_the_wrong_size_are_refused_and_the_right_one_decodes() {
        // Whole pictures the decoder would read: a refusal can only come from the header check.
        let dir =
            std::env::temp_dir().join(format!("sunrise-lightmap-pack-{}", std::process::id()));
        let files = [
            ("too_wide.png", png(MAX_WIDTH + 1, 32)),
            ("too_tall.png", png(16, 33)),
            ("not_png.png", b"not a png at all, just bytes".to_vec()),
            ("ok.png", png(16, 64)),
        ];
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let root = ResourceSource::directory(dir.clone());
        for name in ["too_wide.png", "too_tall.png", "not_png.png"] {
            assert!(decode(&root, name).is_none(), "{name}");
        }
        assert_eq!(decode(&root, "ok.png").map(|i| (i.0, i.1)), Some((16, 64)));
        assert!(
            crate::hud::decode_image_bytes(&files[1].1).is_ok(),
            "the decoder takes it"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
