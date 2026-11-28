//! Custom lightmaps: `optifine/lightmap/world<N>.png` (`world-1` the Nether, `world0` the
//! Overworld, `world1` the End), and beside it `world<N>_rain.png` and `world<N>_thunder.png` for
//! the weather. A world with one has its light texture (the 16x16 table of sky light by block
//! light every world shader reads) taken from the picture instead of computed.
//!
//! A picture is any width and exactly 32 or 64 pixels tall. Its rows 0 to 15 are the sky light at levels
//! 0 to 15 and rows 16 to 31 the block light; across, a column is a moment: the sky's from its
//! darkest (left) to its brightest (right), the block light's from the dimmest flicker to the
//! brightest. A frame takes one column of each, blended between the two nearest pixel columns,
//! and adds them per level: sky level `s` and block level `b` light as `sky[s] + block[b]`. With
//! night vision on (any strength: the effect, its fading pulse or conduit water vision), a picture
//! 64 tall is read from rows 32 to 63 instead; with a 32-tall one the game computes the texture.
//!
//! In a world with weather (not the Nether or the End), a pack's rain and thunder pictures are
//! blended in: the base picture by `1 - rain`, the rain one by `rain - thunder` and the thunder
//! one by `thunder`, the thunder strength being the storm's (already scaled by the rain), so the
//! weights always sum to one.
//!
//! The brightness option lifts the dark end as 26.3's lightmap does: by the brightest channel
//! (`1 - (1 - max)^4`, the colour scaled to it), mixed in by the option, so a tinted picture keeps
//! its channels' ratios.
//!
//! Written from OptiFine's public documentation of the format (the Custom Lightmaps page of
//! OptiDocs) and checked against OptiFine's behaviour.

use std::collections::BTreeMap;
use sunrise_assets::resource_source::ResourceSource;

/// The light texture's side: light levels 0 to 15.
pub const LEVELS: usize = 16;

/// A light texture: RGBA rows of sky level by block level (`(sky * 16 + block) * 4`).
pub type Pixels = [u8; LEVELS * LEVELS * 4];

/// The worlds that have a lightmap: the game only asks for these.
pub const WORLDS: [i32; 3] = [-1, 0, 1];

/// The lightmap files of every world a pack gives one, by world number.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Files {
    pub worlds: BTreeMap<i32, WorldFiles>,
}

/// One world's pictures, as resource paths.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorldFiles {
    pub base: String,
    pub rain: Option<String>,
    pub thunder: Option<String>,
}

impl Files {
    /// The lightmaps of the overlay at `root`; `optifine/` wins over the legacy `mcpatcher/`.
    /// Only the exact names of [`WORLDS`] are looked for (`world0.png`, never `world+0.png`).
    pub fn scan(root: &ResourceSource) -> Self {
        let mut out = Self::default();
        for world in WORLDS {
            for top in ["optifine", "mcpatcher"] {
                let at = |suffix: &str| {
                    let path = format!("assets/minecraft/{top}/lightmap/world{world}{suffix}.png");
                    root.contains(&path).then_some(path)
                };
                if let Some(base) = at("") {
                    out.worlds.insert(
                        world,
                        WorldFiles {
                            base,
                            rain: at("_rain"),
                            thunder: at("_thunder"),
                        },
                    );
                    break;
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.worlds.is_empty()
    }
}

/// What a frame's light texture depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// The sky's brightness, 0 (night) to 1 (noon): the sky light factor.
    pub sky: f32,
    /// Lightning struck lately (the sky flashes): the sky column is the brightest.
    pub lightning: bool,
    /// The block light's flicker, about -0.1 to 0.1 around its rest.
    pub flicker: f32,
    /// The brightness option, 0 (moody) to 1 (bright).
    pub gamma: f32,
    pub night_vision: bool,
    /// The darkness effect's pulse, taken off every channel.
    pub darkness: f32,
    /// The rain and thunder strength, 0 to 1.
    pub rain: f32,
    pub thunder: f32,
}

/// A decoded lightmap picture: linear RGB per pixel, 0 to 1, in rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    width: usize,
    height: usize,
    rgb: Vec<[f32; 3]>,
}

impl Picture {
    /// Whether a picture `width` by `height` is a lightmap: any width, 32 or 64 tall.
    pub fn valid_size(width: u32, height: u32) -> bool {
        width > 0 && (height == 32 || height == 64)
    }

    /// The picture from its RGBA bytes; none when it is not a lightmap's size
    /// ([`Self::valid_size`]) or the bytes are not `width * height` pixels.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        if !Self::valid_size(width, height) {
            return None;
        }
        let (width, height) = (width as usize, height as usize);
        if rgba.len() != width * height * 4 {
            return None;
        }
        let rgb = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| std::array::from_fn(|k| f32::from(p[k]) / 255.0))
            .collect();
        Some(Self { width, height, rgb })
    }

    /// The 16 rows from `top` at column `x` (in pixels, fractional), each blended between its
    /// two nearest pixel columns.
    fn column(&self, x: f32, top: usize) -> [[f32; 3]; LEVELS] {
        let (lo, hi) = (x.floor() as usize, (x.ceil() as usize).min(self.width - 1));
        std::array::from_fn(|row| {
            let at = |c: usize| self.rgb[(top + row) * self.width + c];
            if lo == hi {
                return at(lo);
            }
            let t = x - lo as f32;
            let (a, b) = (at(lo), at(hi));
            std::array::from_fn(|k| a[k] * (1.0 - t) + b[k] * t)
        })
    }

    /// Whether the picture serves `input`: with night vision on, only one with its rows does.
    pub fn serves(&self, input: &Input) -> bool {
        !input.night_vision || self.height >= 4 * LEVELS
    }

    /// The light texture this picture gives for `input`; none when it does not serve it
    /// ([`Self::serves`]).
    pub fn pixels(&self, input: &Input) -> Option<Pixels> {
        if !self.serves(input) {
            return None;
        }
        let top = if input.night_vision { 2 * LEVELS } else { 0 };
        let last = (self.width - 1) as f32;
        let sky = if input.lightning {
            1.0
        } else {
            (7.0 / 6.0 * (input.sky - 0.2)).clamp(0.0, 1.0)
        };
        let sky = self.column(sky * last, top);
        let block = self.column((input.flicker + 0.5).clamp(0.0, 1.0) * last, top + LEVELS);
        let gamma = input.gamma.clamp(0.0, 1.0);
        let mut out = [0u8; LEVELS * LEVELS * 4];
        for (s, sky) in sky.iter().enumerate() {
            for (b, block) in block.iter().enumerate() {
                let mut c: [f32; 3] =
                    std::array::from_fn(|k| (sky[k] + block[k] - input.darkness).clamp(0.0, 1.0));
                // 26.3's brightness curve: the colour scaled so its brightest channel follows
                // `1 - (1 - max)^4`, mixed in by the option.
                let max = c[0].max(c[1]).max(c[2]).max(1e-6);
                let inverted = 1.0 - max;
                let scale = (1.0 - inverted * inverted * inverted * inverted) / max;
                for v in &mut c {
                    *v += (*v * scale - *v) * gamma;
                }
                let i = (s * LEVELS + b) * 4;
                for k in 0..3 {
                    out[i + k] = (c[k].clamp(0.0, 1.0) * 255.0) as u8;
                }
                out[i + 3] = 255;
            }
        }
        Some(out)
    }
}

/// One world's lightmap: its picture, and the rain and thunder ones it blends to.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldLightmap {
    pub base: Picture,
    pub rain: Option<Picture>,
    pub thunder: Option<Picture>,
}

impl WorldLightmap {
    /// The light texture of world `world` for `input`: the base picture's, blended with the rain
    /// and thunder pictures by the weather where the world has weather. A missing thunder picture
    /// is the rain one, and a missing rain picture the base one; under night vision a weather
    /// picture without its rows is the base one too. None when the base picture does not serve
    /// (night vision without its rows): the game computes the whole texture then.
    pub fn pixels(&self, world: i32, input: &Input) -> Option<Pixels> {
        if !self.base.serves(input) {
            return None;
        }
        let weathered = !matches!(world, -1 | 1) && (self.rain.is_some() || self.thunder.is_some());
        if !weathered || (input.rain <= 0.0 && input.thunder <= 0.0) {
            return self.base.pixels(input);
        }
        // A weather picture that does not serve (night vision without its rows) is the base; a
        // missing thunder picture is the rain one, a missing rain picture the base.
        let rain_picture = match &self.rain {
            Some(picture) if picture.serves(input) => picture,
            _ => &self.base,
        };
        let thunder_picture = match &self.thunder {
            Some(picture) if picture.serves(input) => picture,
            Some(_) => &self.base,
            None => rain_picture,
        };
        // A NaN strength reads as none (`clamp` would panic on a NaN bound).
        let none_if_nan = |v: f32| if v.is_nan() { 0.0 } else { v };
        let rain = none_if_nan(input.rain).clamp(0.0, 1.0);
        let thunder = none_if_nan(input.thunder).clamp(0.0, rain);
        let weights = [1.0 - rain, rain - thunder, thunder];
        let mut sum = [0f32; LEVELS * LEVELS * 3];
        for (picture, weight) in [&self.base, rain_picture, thunder_picture]
            .into_iter()
            .zip(weights)
        {
            if weight <= 0.0 {
                continue;
            }
            let pixels = picture.pixels(input)?;
            for (n, total) in sum.iter_mut().enumerate() {
                *total += f32::from(pixels[n / 3 * 4 + n % 3]) * weight;
            }
        }
        let mut out = [255u8; LEVELS * LEVELS * 4];
        for (n, total) in sum.iter().enumerate() {
            // Rounded: weights that sum to one keep a channel the pictures share.
            out[n / 3 * 4 + n % 3] = total.round().clamp(0.0, 255.0) as u8;
        }
        Some(out)
    }
}

/// A pack's lightmaps, decoded, by world number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lightmaps {
    pub worlds: BTreeMap<i32, WorldLightmap>,
}

impl Lightmaps {
    /// The overlay's lightmaps, with `decode` reading a resource path into its width, height
    /// and RGBA bytes. A world whose picture does not decode, or is too small, has none; a rain
    /// or thunder picture that does not is left out.
    pub fn load(
        files: &Files,
        mut decode: impl FnMut(&str) -> Option<(u32, u32, Vec<u8>)>,
    ) -> Self {
        let mut picture = |path: &str| {
            let (width, height, rgba) = decode(path)?;
            let picture = Picture::from_rgba(width, height, &rgba);
            if picture.is_none() {
                log::warn!("{path}: a lightmap is 32 or 64 pixels tall");
            }
            picture
        };
        let worlds = files
            .worlds
            .iter()
            .filter_map(|(&world, f)| {
                let base = picture(&f.base)?;
                Some((
                    world,
                    WorldLightmap {
                        base,
                        rain: f.rain.as_deref().and_then(&mut picture),
                        thunder: f.thunder.as_deref().and_then(&mut picture),
                    },
                ))
            })
            .collect();
        Self { worlds }
    }

    pub fn is_empty(&self) -> bool {
        self.worlds.is_empty()
    }

    /// Whether world `world` has a lightmap.
    pub fn has(&self, world: i32) -> bool {
        self.worlds.contains_key(&world)
    }

    /// World `world`'s light texture for `input`; none without a lightmap that serves.
    pub fn pixels(&self, world: i32, input: &Input) -> Option<Pixels> {
        self.worlds.get(&world)?.pixels(world, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `width` by `height` picture whose pixel at (x, y) is `f(x, y)` (grey).
    fn picture(width: u32, height: u32, f: impl Fn(u32, u32) -> u8) -> Picture {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let v = f(x, y);
                rgba.extend([v, v, v, 255]);
            }
        }
        Picture::from_rgba(width, height, &rgba).unwrap()
    }

    fn input() -> Input {
        Input {
            sky: 1.0,
            lightning: false,
            flicker: 0.0,
            gamma: 0.0,
            night_vision: false,
            darkness: 0.0,
            rain: 0.0,
            thunder: 0.0,
        }
    }

    fn red(p: &Pixels, sky: usize, block: usize) -> u8 {
        p[(sky * LEVELS + block) * 4]
    }

    #[test]
    fn lightmaps_are_any_width_and_32_or_64_tall() {
        assert!(
            Picture::from_rgba(1, 32, &[0; 32 * 4]).is_some(),
            "a narrow palette"
        );
        assert!(Picture::from_rgba(16, 64, &[0; 16 * 64 * 4]).is_some());
        assert!(Picture::from_rgba(0, 32, &[]).is_none());
        for height in [31, 33, 63, 65] {
            assert!(Picture::from_rgba(16, height, &vec![0; 16 * height as usize * 4]).is_none());
        }
        assert!(Picture::from_rgba(16, 32, &[0; 16 * 32 * 4 - 1]).is_none());
    }

    #[test]
    fn a_one_pixel_wide_palette_is_constant() {
        let p = picture(1, 32, |_, y| if y < 16 { y as u8 * 10 } else { 5 });
        for sky in [0.0, 0.5, 1.0] {
            let out = p
                .pixels(&Input {
                    sky,
                    flicker: 0.3,
                    ..input()
                })
                .unwrap();
            assert_eq!(red(&out, 3, 0), 35);
        }
    }

    #[test]
    fn sky_and_block_rows_add_up() {
        // Sky rows at 10 per level, block rows at 5 per level.
        let p = picture(16, 32, |_, y| {
            if y < 16 {
                y as u8 * 10
            } else {
                (y - 16) as u8 * 5
            }
        });
        let out = p.pixels(&input()).unwrap();
        assert_eq!(red(&out, 0, 0), 0);
        assert_eq!(red(&out, 3, 0), 30);
        assert_eq!(red(&out, 0, 4), 20);
        assert_eq!(red(&out, 3, 4), 50);
        assert_eq!(out[3], 255);
    }

    #[test]
    fn the_sky_column_follows_the_sky_light() {
        // Each column is its own brightness: x * 17 (0 to 255).
        let p = picture(16, 32, |x, y| if y < 16 { x as u8 * 17 } else { 0 });
        let at = |sky| red(&p.pixels(&Input { sky, ..input() }).unwrap(), 15, 0);
        // A sky factor of 1 (noon) is 14/15 of the way across; only a sky past it, or a flash,
        // reaches the right-hand column.
        assert!((237..=239).contains(&at(1.0)), "{}", at(1.0));
        assert_eq!(at(1.1), 255);
        assert_eq!(at(0.2), 0, "a sky at 0.2 and below is the left-hand one");
        assert_eq!(at(0.0), 0);
        // Lightning lights the sky fully whatever the time.
        let flash = p
            .pixels(&Input {
                sky: 0.0,
                lightning: true,
                ..input()
            })
            .unwrap();
        assert_eq!(red(&flash, 15, 0), 255);
    }

    #[test]
    fn columns_are_blended_between_pixels() {
        // Two columns wide in effect: left 0, everything right of it 255.
        let p = picture(17, 32, |x, y| if y < 16 && x > 0 { 255 } else { 0 });
        // A sky column at x = 0.5 of 16 columns: halfway between pixels 0 and 1.
        let sky = 0.5 / 16.0 * 6.0 / 7.0 + 0.2;
        let v = red(&p.pixels(&Input { sky, ..input() }).unwrap(), 0, 0);
        assert!((126..=128).contains(&v), "{v}");
    }

    #[test]
    fn darkness_and_gamma() {
        let p = picture(16, 32, |_, y| if y < 16 { 128 } else { 0 });
        let dark = p
            .pixels(&Input {
                darkness: 0.25,
                ..input()
            })
            .unwrap();
        assert_eq!(red(&dark, 0, 0), ((128.0 / 255.0 - 0.25) * 255.0) as u8);
        let bright = p
            .pixels(&Input {
                gamma: 1.0,
                ..input()
            })
            .unwrap();
        let v = 128.0f32 / 255.0;
        assert_eq!(
            red(&bright, 0, 0),
            ((1.0 - (1.0 - v).powi(4)) * 255.0) as u8
        );
    }

    #[test]
    fn brightness_keeps_a_tinted_palettes_channel_ratios() {
        // Sky rows (0.5, 0.25, 0): the brightest channel's curve scales all three.
        let mut rgba = Vec::new();
        for y in 0..32 {
            for _ in 0..16 {
                rgba.extend(if y < 16 {
                    [128, 64, 0, 255]
                } else {
                    [0, 0, 0, 255]
                });
            }
        }
        let p = Picture::from_rgba(16, 32, &rgba).unwrap();
        let out = p
            .pixels(&Input {
                gamma: 1.0,
                ..input()
            })
            .unwrap();
        let (r, g, b) = (out[0], out[1], out[2]);
        assert_eq!(b, 0);
        assert!(r > 128 && g > 64, "brightened: {r} {g}");
        let ratio = f32::from(r) / f32::from(g);
        assert!((ratio - 2.0).abs() < 0.05, "ratio {ratio}");
        // At half the option, halfway between the picture and the curve.
        let half = p
            .pixels(&Input {
                gamma: 0.5,
                ..input()
            })
            .unwrap();
        assert!((i32::from(half[0]) - (128 + i32::from(r)) / 2).abs() <= 1);
    }

    #[test]
    fn night_vision_uses_the_base_for_weather_pictures_without_its_rows() {
        let tall = picture(16, 64, |_, y| if (32..48).contains(&y) { 200 } else { 0 });
        let short = picture(16, 32, |_, y| if y < 16 { 50 } else { 0 });
        let w = WorldLightmap {
            base: tall,
            rain: Some(short.clone()),
            thunder: None,
        };
        let nv = |rain| Input {
            night_vision: true,
            rain,
            ..input()
        };
        assert_eq!(red(&w.pixels(0, &nv(0.5)).unwrap(), 0, 0), 200);
        assert_eq!(red(&w.pixels(0, &nv(1.0)).unwrap(), 0, 0), 200);
        // A base without the rows: the game's texture for the whole frame.
        let w = WorldLightmap {
            base: short,
            rain: None,
            thunder: None,
        };
        assert!(w.pixels(0, &nv(0.0)).is_none());
    }

    #[test]
    fn night_vision_reads_its_own_rows_or_none() {
        assert!(
            picture(16, 32, |_, _| 0)
                .pixels(&Input {
                    night_vision: true,
                    ..input()
                })
                .is_none()
        );
        let p = picture(16, 64, |_, y| if (32..48).contains(&y) { 200 } else { 0 });
        let out = p
            .pixels(&Input {
                night_vision: true,
                ..input()
            })
            .unwrap();
        assert_eq!(red(&out, 5, 5), 200);
        assert_eq!(red(&p.pixels(&input()).unwrap(), 5, 5), 0);
    }

    fn world(base: u8, rain: Option<u8>, thunder: Option<u8>) -> WorldLightmap {
        let flat = |v: u8| picture(16, 32, move |_, y| if y < 16 { v } else { 0 });
        WorldLightmap {
            base: flat(base),
            rain: rain.map(flat),
            thunder: thunder.map(flat),
        }
    }

    #[test]
    fn weather_blends_the_rain_and_thunder_pictures() {
        let w = world(200, Some(100), Some(0));
        let at = |world, rain, thunder| {
            red(
                &w.pixels(
                    world,
                    &Input {
                        rain,
                        thunder,
                        ..input()
                    },
                )
                .unwrap(),
                0,
                0,
            )
        };
        assert_eq!(at(0, 0.0, 0.0), 200);
        assert_eq!(at(0, 1.0, 0.0), 100);
        assert_eq!(at(0, 0.5, 0.0), 150);
        assert_eq!(at(0, 1.0, 1.0), 0);
        // The weights sum to one with rain and thunder both partway in.
        assert_eq!(at(0, 0.5, 0.25), (200.0 * 0.5 + 100.0 * 0.25) as u8);
        let same = world(100, Some(100), Some(100));
        let out = same
            .pixels(
                0,
                &Input {
                    rain: 0.25,
                    thunder: 0.1875,
                    ..input()
                },
            )
            .unwrap();
        assert!((99..=100).contains(&red(&out, 0, 0)), "{}", red(&out, 0, 0));
        // No weather in the Nether or the End.
        assert_eq!(at(-1, 1.0, 1.0), 200);
        assert_eq!(at(1, 1.0, 0.0), 200);
    }

    #[test]
    fn a_nan_weather_strength_reads_as_none() {
        let w = world(200, Some(100), Some(0));
        for (rain, thunder) in [(f32::NAN, 0.5), (0.5, f32::NAN), (f32::NAN, f32::NAN)] {
            assert!(
                w.pixels(
                    0,
                    &Input {
                        rain,
                        thunder,
                        ..input()
                    }
                )
                .is_some()
            );
        }
        let out = w
            .pixels(
                0,
                &Input {
                    rain: f32::NAN,
                    thunder: f32::NAN,
                    ..input()
                },
            )
            .unwrap();
        assert_eq!(red(&out, 0, 0), 200);
    }

    #[test]
    fn a_faint_storm_keeps_the_full_weight() {
        let white = world(255, Some(255), Some(255));
        let faint = Input {
            rain: 0.00015,
            thunder: 0.000075,
            ..input()
        };
        let out = white.pixels(0, &faint).unwrap();
        assert_eq!(&out[..3], &[255, 255, 255]);
    }

    #[test]
    fn a_thunder_picture_without_night_vision_rows_is_the_base_not_the_rain_one() {
        let tall = |v: u8| {
            picture(
                16,
                64,
                move |_, y| if (32..48).contains(&y) { v } else { 0 },
            )
        };
        let short = picture(16, 32, |_, y| if y < 16 { 50 } else { 0 });
        let w = WorldLightmap {
            base: tall(200),
            rain: Some(tall(100)),
            thunder: Some(short),
        };
        let storm = Input {
            night_vision: true,
            rain: 1.0,
            thunder: 1.0,
            ..input()
        };
        assert_eq!(red(&w.pixels(0, &storm).unwrap(), 0, 0), 200);
    }

    #[test]
    fn a_missing_thunder_picture_is_the_rain_one() {
        let w = world(200, Some(100), None);
        let out = w
            .pixels(
                0,
                &Input {
                    rain: 1.0,
                    thunder: 1.0,
                    ..input()
                },
            )
            .unwrap();
        assert_eq!(red(&out, 0, 0), 100);
        let only_base = world(200, None, None);
        let out = only_base
            .pixels(
                0,
                &Input {
                    rain: 1.0,
                    thunder: 1.0,
                    ..input()
                },
            )
            .unwrap();
        assert_eq!(red(&out, 0, 0), 200);
    }

    #[test]
    fn files_pair_each_world_with_its_weather() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-optifine-lightmap-{}", std::process::id()));
        let at = |p: &str| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        };
        at("assets/minecraft/optifine/lightmap/world0.png");
        at("assets/minecraft/optifine/lightmap/world0_rain.png");
        at("assets/minecraft/optifine/lightmap/world-1.png");
        at("assets/minecraft/mcpatcher/lightmap/world-1.png");
        at("assets/minecraft/mcpatcher/lightmap/world1.png");
        at("assets/minecraft/optifine/lightmap/notes.txt");
        at("assets/minecraft/optifine/lightmap/world7.png");
        at("assets/minecraft/mcpatcher/lightmap/world+0.png");
        at("assets/minecraft/optifine/lightmap/world+1.png");
        let files = Files::scan(&ResourceSource::directory(dir.clone()));
        std::fs::remove_dir_all(&dir).ok();
        let o = "assets/minecraft/optifine/lightmap";
        let m = "assets/minecraft/mcpatcher/lightmap";
        assert_eq!(files.worlds.keys().copied().collect::<Vec<_>>(), [-1, 0, 1]);
        assert_eq!(files.worlds[&0].base, format!("{o}/world0.png"));
        assert_eq!(files.worlds[&0].rain, Some(format!("{o}/world0_rain.png")));
        assert_eq!(files.worlds[&0].thunder, None);
        assert_eq!(
            files.worlds[&-1].base,
            format!("{o}/world-1.png"),
            "optifine/ first"
        );
        assert_eq!(files.worlds[&1].base, format!("{m}/world1.png"));
    }

    #[test]
    fn load_drops_what_does_not_decode() {
        let files = Files {
            worlds: [
                (
                    0,
                    WorldFiles {
                        base: "a".into(),
                        rain: Some("bad".into()),
                        thunder: None,
                    },
                ),
                (
                    1,
                    WorldFiles {
                        base: "small".into(),
                        rain: None,
                        thunder: None,
                    },
                ),
            ]
            .into(),
        };
        let maps = Lightmaps::load(&files, |path| match path {
            "a" => Some((16, 32, vec![0; 16 * 32 * 4])),
            "small" => Some((8, 8, vec![0; 8 * 8 * 4])),
            _ => None,
        });
        assert!(maps.has(0) && !maps.has(1));
        assert!(maps.worlds[&0].rain.is_none());
    }
}
