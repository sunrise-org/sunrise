//! OptiFine custom animations (`sunrise_optifine::anim`) on the textures this atlas packs: each
//! animation's area, placed where its target texture sits in the atlas, re-uploaded as its frames
//! turn, one game tick at a time. An animation of a texture the atlas does not pack, or packs at
//! another size than the file's (a reduced model texture), is left out.

use super::AnimatedPlacement;
use crate::resource_source::ResourceSource;
use std::collections::HashMap;
use sunrise_optifine::anim::{Animation, Rule};

/// An animation and the atlas area it draws into.
#[derive(Clone, Debug)]
pub struct CustomAnimation {
    pub area: AnimatedPlacement,
    pub animation: Animation,
}

/// The atlas's sprite name of a texture's resource path: `assets/minecraft/textures/x.png` is
/// `x`, another namespace's `ns:x`.
fn sprite_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/")?;
    let (namespace, rest) = rest.split_once('/')?;
    let name = rest.strip_prefix("textures/")?.strip_suffix(".png")?;
    Some(if namespace == "minecraft" {
        name.to_owned()
    } else {
        format!("{namespace}:{name}")
    })
}

/// The animations of the overlay at `root` whose targets `placed` holds at their own size, with
/// each first frame written into `pixels` (the atlas, `size`).
pub(super) fn build(
    root: &ResourceSource,
    placed: &HashMap<String, AnimatedPlacement>,
    pixels: &mut [u8],
    size: [u32; 2],
) -> Vec<CustomAnimation> {
    let mut out = Vec::new();
    for rule in Rule::scan(root) {
        let Some(target) = sprite_name(&rule.to).and_then(|name| placed.get(&name)) else {
            log::warn!(
                "{}: {} is not a texture Sunrise animates",
                rule.file,
                rule.to
            );
            continue;
        };
        // The texture as the atlas packs it must be the file's size: the area is in its pixels.
        if super::image::png_size(root, &rule.to) != Some((target.w, target.h)) {
            log::warn!("{}: {} is packed at another size", rule.file, rule.to);
            continue;
        }
        let strip = match root
            .read(&rule.from)
            .map_err(Into::into)
            .and_then(|bytes| super::decode_image_bytes(&bytes))
        {
            Ok(strip) => strip,
            Err(e) => {
                log::warn!("{}: {e:#}", rule.from);
                continue;
            }
        };
        let (x, y, w, h) = (rule.x, rule.y, rule.w, rule.h);
        let Some(animation) =
            Animation::new(rule, (strip.0, strip.1, &strip.2), (target.w, target.h))
        else {
            continue;
        };
        let area = AnimatedPlacement {
            x: target.x + x,
            y: target.y + y,
            w,
            h,
            border: super::AtlasBorder::None,
        };
        if let Some(first) = animation.first() {
            blit(pixels, size, area, &first);
        }
        out.push(CustomAnimation { area, animation });
    }
    out
}

/// Writes `area`'s RGBA rows into the atlas `pixels`, `size` across and down.
fn blit(pixels: &mut [u8], [width, height]: [u32; 2], area: AnimatedPlacement, rows: &[u8]) {
    if area.x + area.w > width || area.y + area.h > height {
        return;
    }
    let row = (area.w * 4) as usize;
    for (r, line) in rows.chunks_exact(row).take(area.h as usize).enumerate() {
        let at = (((area.y as usize + r) * width as usize) + area.x as usize) * 4;
        pixels[at..at + row].copy_from_slice(line);
    }
}

/// The running animations, and the game tick they were last advanced to.
#[derive(Default)]
pub struct Running {
    animations: Vec<CustomAnimation>,
    tick: Option<u64>,
}

/// Ticks one frame catches up at most (a stall, or a jump in the game's tick count).
const CATCH_UP: u64 = 20;

impl Running {
    pub fn new(animations: Vec<CustomAnimation>) -> Self {
        Self {
            animations,
            tick: None,
        }
    }

    /// Advances every animation to game tick `tick`, handing each area whose pixels changed to
    /// `upload` once, with its newest pixels.
    pub fn advance(&mut self, tick: u64, mut upload: impl FnMut(AnimatedPlacement, &[u8])) {
        let last = self.tick.replace(tick);
        let ticks = last.map_or(0, |last| tick.saturating_sub(last).min(CATCH_UP));
        if ticks == 0 {
            return;
        }
        for a in &mut self.animations {
            let mut newest = None;
            for _ in 0..ticks {
                if let Some(pixels) = a.animation.tick() {
                    newest = Some(pixels);
                }
            }
            if let Some(pixels) = newest {
                upload(a.area, &pixels);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_names_follow_the_atlas() {
        assert_eq!(
            sprite_name("assets/minecraft/textures/entity/cow/cow.png").as_deref(),
            Some("entity/cow/cow")
        );
        assert_eq!(
            sprite_name("assets/mymod/textures/gui/panel.png").as_deref(),
            Some("mymod:gui/panel")
        );
        assert_eq!(sprite_name("assets/minecraft/optifine/anim/a.png"), None);
    }

    #[test]
    fn blit_writes_the_area_rows() {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        let area = AnimatedPlacement {
            x: 1,
            y: 2,
            w: 2,
            h: 1,
            border: crate::hud::AtlasBorder::None,
        };
        blit(&mut pixels, [4, 4], area, &[9; 8]);
        let row2 = &pixels[2 * 16..3 * 16];
        assert_eq!(row2, &[0, 0, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9, 0, 0, 0, 0]);
        assert!(pixels[..32].iter().all(|&b| b == 0));
    }

    #[test]
    fn advancing_catches_up_ticks_and_uploads_once() {
        let rule = Rule::parse(
            "assets/minecraft/optifine/anim/a.properties",
            &sunrise_optifine::properties::Properties::parse(
                "from=a.png\nto=textures/b.png\nx=0\ny=0\nw=1\nh=1",
            ),
        )
        .unwrap();
        let strip: Vec<u8> = [[0, 0, 0, 255], [10, 0, 0, 255], [20, 0, 0, 255]].concat();
        let animation = Animation::new(rule, (1, 3, &strip), (1, 1)).unwrap();
        let area = AnimatedPlacement {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            border: crate::hud::AtlasBorder::None,
        };
        let mut running = Running::new(vec![CustomAnimation { area, animation }]);
        let mut uploads = Vec::new();
        running.advance(100, |_, p| uploads.push(p[0]));
        assert!(uploads.is_empty(), "the first frame only notes the tick");
        running.advance(102, |_, p| uploads.push(p[0]));
        assert_eq!(uploads, [20], "two ticks, one upload of the newest frame");
        running.advance(102, |_, p| uploads.push(p[0]));
        assert_eq!(uploads, [20]);
    }
}
