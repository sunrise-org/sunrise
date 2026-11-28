//! Custom animations: `optifine/anim/**/*.properties`, each animating an area of a texture with
//! frames from a strip, a tick at a time. Unlike a `.png.mcmeta` animation, it can animate any
//! texture (an entity's, the GUI's, the sky's), and only part of one.
//!
//! A file names the strip (`from`), the texture it draws into (`to`), and the area (`x`, `y`, `w`,
//! `h`, in the texture's pixels). The strip is `w` wide and holds its frames from the top, each `h`
//! tall; a strip of another width is scaled to `w` across, its height in proportion. By default
//! the frames show in order, each for `duration` ticks (1); `tile.<n>` and `duration.<n>` give the
//! `n`th step's frame and ticks. With `interpolate=true` each tick between two steps blends the
//! frame toward the next one, every `skip`th tick when `skip` is above 1.
//!
//! Written from OptiFine's public documentation of the format (`OptiFineDoc/doc/anim.properties`
//! and the Custom Animations page of OptiDocs) and checked against OptiFine's behaviour.

use sunrise_assets::resource_source::ResourceSource;

use crate::paths::{properties_files, resolve};
use crate::properties::Properties;

/// One animation file, read: where its strip and target are, and its steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The properties file, for warnings.
    pub file: String,
    /// The strip's resource path.
    pub from: String,
    /// The target texture's resource path.
    pub to: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Each step's frame index into the strip and its ticks (at least 1). Empty until the strip's
    /// frame count is known, when no `tile.<n>` lists them ([`Animation::new`]).
    pub steps: Vec<(u32, u32)>,
    /// `duration`: the ticks of a step without `duration.<n>`.
    pub duration: u32,
    pub interpolate: bool,
    pub skip: u32,
}

/// The texture path a value names: resolved like OptiFine's other paths, `.png` added when it
/// has no extension.
fn texture(file: &str, value: &str) -> Option<String> {
    let path = resolve(file, value)?;
    let name = path.rsplit('/').next().unwrap_or(&path);
    Some(if name.contains('.') {
        path
    } else {
        format!("{path}.png")
    })
}

fn number(props: &Properties, key: &str) -> Option<i64> {
    props.get(key).and_then(|v| v.trim().parse().ok())
}

impl Rule {
    /// The rule of the file at `file`; none when it lacks `from` or `to`, a path leaves the pack,
    /// or the area is not a positive size at a non-negative place.
    pub fn parse(file: &str, props: &Properties) -> Option<Self> {
        let from = texture(file, props.get("from")?)?;
        let to = texture(file, props.get("to")?)?;
        let coordinate = |key| u32::try_from(number(props, key)?).ok();
        let size = |key| coordinate(key).filter(|&v| v > 0);
        let (x, y, w, h) = (coordinate("x")?, coordinate("y")?, size("w")?, size("h")?);
        let ticks = |value: Option<i64>, default: u32| {
            value.map_or(default, |v| u32::try_from(v.max(1)).unwrap_or(u32::MAX))
        };
        let duration = ticks(number(props, "duration"), 1);
        let steps = (0..)
            .map_while(|n| {
                let tile = u32::try_from(number(props, &format!("tile.{n}"))?).ok()?;
                Some((
                    tile,
                    ticks(number(props, &format!("duration.{n}")), duration),
                ))
            })
            .collect();
        Some(Self {
            file: file.to_owned(),
            from,
            to,
            x,
            y,
            w,
            h,
            steps,
            duration,
            interpolate: props
                .get("interpolate")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
            skip: number(props, "skip").map_or(0, |v| u32::try_from(v.max(0)).unwrap_or(0)),
        })
    }

    /// Every animation of the overlay at `root`, in file order. A file that does not read is
    /// left out with a warning.
    pub fn scan(root: &ResourceSource) -> Vec<Self> {
        properties_files(root, "anim")
            .into_iter()
            .filter_map(|file| {
                let text = root.read_to_string(&file).ok()?;
                let rule = Self::parse(&file, &Properties::parse(&text));
                if rule.is_none() {
                    log::warn!("{file}: an animation needs from, to, x, y, w and h");
                }
                rule
            })
            .collect()
    }
}

/// `rgba` (`width` by `height`) scaled to `w` across, its height in proportion, bilinearly.
fn scale_to_width(width: u32, height: u32, rgba: &[u8], w: u32) -> (u32, Vec<u8>) {
    if width == w {
        return (height, rgba.to_vec());
    }
    let h = ((u64::from(height) * u64::from(w)) / u64::from(width)).max(1) as u32;
    let (sx, sy) = (width as f32 / w as f32, height as f32 / h as f32);
    let at = |x: u32, y: u32, k: usize| f32::from(rgba[((y * width + x) * 4) as usize + k]);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (height - 1) as f32);
        let (y0, ty) = (fy.floor() as u32, fy.fract());
        let y1 = (y0 + 1).min(height - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (width - 1) as f32);
            let (x0, tx) = (fx.floor() as u32, fx.fract());
            let x1 = (x0 + 1).min(width - 1);
            for k in 0..4 {
                let top = at(x0, y0, k) * (1.0 - tx) + at(x1, y0, k) * tx;
                let bottom = at(x0, y1, k) * (1.0 - tx) + at(x1, y1, k) * tx;
                out.push((top * (1.0 - ty) + bottom * ty).round() as u8);
            }
        }
    }
    (h, out)
}

/// A running animation: its rule, the strip's frames, and where it is.
#[derive(Clone, Debug)]
pub struct Animation {
    pub rule: Rule,
    /// The strip, `rule.w` wide, a whole number of `rule.h` tall frames.
    frames: Vec<u8>,
    step: usize,
    /// Ticks spent in each step so far.
    counters: Vec<u32>,
}

impl Animation {
    /// The animation of `rule` with its strip (`width` by `height` RGBA) and the target texture's
    /// size; none when the strip is not a whole number of frames, has none, or the area is outside
    /// the target.
    pub fn new(
        mut rule: Rule,
        (width, height, rgba): (u32, u32, &[u8]),
        (target_w, target_h): (u32, u32),
    ) -> Option<Self> {
        if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
            return None;
        }
        if rule.x.checked_add(rule.w)? > target_w || rule.y.checked_add(rule.h)? > target_h {
            log::warn!("{}: the area is outside {}", rule.file, rule.to);
            return None;
        }
        let (strip_h, frames) = scale_to_width(width, height, rgba, rule.w);
        if strip_h % rule.h != 0 {
            log::warn!(
                "{}: {} is not a whole number of frames",
                rule.file,
                rule.from
            );
            return None;
        }
        let count = strip_h / rule.h;
        if rule.steps.is_empty() {
            rule.steps = (0..count).map(|i| (i, rule.duration)).collect();
        }
        if rule.steps.is_empty() {
            return None;
        }
        Some(Self {
            counters: vec![0; rule.steps.len()],
            rule,
            frames,
            step: 0,
        })
    }

    /// The `index`th frame's pixels, none past the strip.
    fn frame(&self, index: u32) -> Option<&[u8]> {
        let size = (self.rule.w * self.rule.h * 4) as usize;
        let start = index as usize * size;
        self.frames.get(start..start + size)
    }

    /// The pixels shown first, before any tick.
    pub fn first(&self) -> Option<Vec<u8>> {
        self.frame(self.rule.steps[0].0).map(<[u8]>::to_vec)
    }

    /// One tick: the area's new pixels (RGBA rows, `w * h * 4` bytes) when they change this tick.
    /// A step lasts its ticks; with interpolation, each tick in between (every `skip`th) blends
    /// its frame toward the next step's.
    pub fn tick(&mut self) -> Option<Vec<u8>> {
        let steps = self.rule.steps.len();
        self.counters[self.step] += 1;
        if self.counters[self.step] >= self.rule.steps[self.step].1 {
            self.counters[self.step] = 0;
            self.step = (self.step + 1) % steps;
        } else if !self.rule.interpolate {
            return None;
        }
        let (index, ticks) = self.rule.steps[self.step];
        let counter = self.counters[self.step];
        if self.rule.interpolate && counter > 0 {
            if self.rule.skip > 1 && !counter.is_multiple_of(self.rule.skip) {
                return None;
            }
            let next = self.rule.steps[(self.step + 1) % steps].0;
            let t = counter as f32 / ticks as f32;
            let (a, b) = (self.frame(index)?, self.frame(next)?);
            return Some(
                a.iter()
                    .zip(b)
                    .map(|(&a, &b)| (f32::from(a) * (1.0 - t) + f32::from(b) * t) as u8)
                    .collect(),
            );
        }
        self.frame(index).map(<[u8]>::to_vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "assets/minecraft/optifine/anim/flag.properties";

    fn rule(text: &str) -> Option<Rule> {
        Rule::parse(FILE, &Properties::parse(text))
    }

    /// A 2x2 strip of `frames` frames, each filled with its index * 10.
    fn strip(frames: u32) -> (u32, u32, Vec<u8>) {
        let mut rgba = Vec::new();
        for f in 0..frames {
            rgba.extend(std::iter::repeat_n([f as u8 * 10, 0, 0, 255], 4).flatten());
        }
        (2, 2 * frames, rgba)
    }

    fn animation(text: &str, frames: u32) -> Animation {
        let (w, h, rgba) = strip(frames);
        Animation::new(rule(text).unwrap(), (w, h, &rgba), (16, 16)).unwrap()
    }

    const BASE: &str =
        "from=./flag_strip.png\nto=textures/entity/banner_base\nx=4\ny=6\nw=2\nh=2\n";

    #[test]
    fn paths_resolve_and_gain_png() {
        let r = rule(BASE).unwrap();
        assert_eq!(r.from, "assets/minecraft/optifine/anim/flag_strip.png");
        assert_eq!(r.to, "assets/minecraft/textures/entity/banner_base.png");
        assert_eq!((r.x, r.y, r.w, r.h), (4, 6, 2, 2));
        let tilde =
            rule("from=~/anim/a\nto=minecraft:textures/gui/x.png\nx=0\ny=0\nw=1\nh=1").unwrap();
        assert_eq!(tilde.from, "assets/minecraft/optifine/anim/a.png");
        assert_eq!(tilde.to, "assets/minecraft/textures/gui/x.png");
    }

    #[test]
    fn incomplete_rules_are_refused() {
        assert!(rule("to=a.png\nx=0\ny=0\nw=1\nh=1").is_none());
        assert!(rule("from=a.png\nto=b.png\nx=0\ny=0\nw=0\nh=1").is_none());
        assert!(rule("from=a.png\nto=b.png\nx=-1\ny=0\nw=1\nh=1").is_none());
        assert!(rule("from=../a.png\nto=b.png\nx=0\ny=0\nw=1\nh=1").is_none());
    }

    #[test]
    fn frames_play_in_order_by_default() {
        let mut a = animation(BASE, 3);
        assert_eq!(a.first().unwrap()[0], 0);
        let reds: Vec<_> = (0..4).map(|_| a.tick().unwrap()[0]).collect();
        assert_eq!(reds, [10, 20, 0, 10]);
    }

    #[test]
    fn tiles_and_durations_set_the_steps() {
        let mut a = animation(
            &format!("{BASE}duration=2\ntile.0=2\ntile.1=0\nduration.1=1\n"),
            3,
        );
        assert_eq!(a.first().unwrap()[0], 20);
        // tile.0 lasts the default 2 ticks, tile.1 one.
        let shown: Vec<_> = (0..5).map(|_| a.tick().map(|p| p[0])).collect();
        assert_eq!(shown, [None, Some(0), Some(20), None, Some(0)]);
    }

    #[test]
    fn interpolation_blends_toward_the_next_step() {
        let mut a = animation(&format!("{BASE}duration=4\ninterpolate=true\n"), 2);
        let shown: Vec<_> = (0..4).map(|_| a.tick().unwrap()[0]).collect();
        // 0 -> 10 over four ticks, then frame 1 itself.
        assert_eq!(shown, [2, 5, 7, 10]);
        let mut skipping = animation(&format!("{BASE}duration=4\ninterpolate=true\nskip=2\n"), 2);
        let shown: Vec<_> = (0..4).map(|_| skipping.tick().map(|p| p[0])).collect();
        assert_eq!(shown, [None, Some(5), None, Some(10)]);
    }

    #[test]
    fn a_strip_must_fit_and_be_whole_frames() {
        let r = rule(BASE).unwrap();
        let (w, h, rgba) = strip(2);
        assert!(
            Animation::new(r.clone(), (w, h, &rgba), (5, 16)).is_none(),
            "x + w past 5"
        );
        assert!(Animation::new(r.clone(), (w, 3, &rgba[..2 * 3 * 4]), (16, 16)).is_none());
        assert!(Animation::new(r, (w, h, &rgba), (16, 16)).is_some());
    }

    #[test]
    fn a_wider_strip_is_scaled_to_the_area() {
        // A 4x8 strip for a 2x2 area: scaled to 2x4, two frames.
        let mut rgba = vec![0u8; 4 * 8 * 4];
        for px in rgba.chunks_mut(4).skip(16) {
            px.copy_from_slice(&[200, 0, 0, 255]);
        }
        let a = Animation::new(rule(BASE).unwrap(), (4, 8, &rgba), (16, 16)).unwrap();
        assert_eq!(a.rule.steps.len(), 2);
        assert_eq!(a.first().unwrap()[0], 0);
    }
}
