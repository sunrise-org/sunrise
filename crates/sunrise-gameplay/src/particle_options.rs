//! The particle an `AreaEffectCloud` draws from its saved `custom_particle`,
//! decoded by the type's own codec (`ParticleTypes.CODEC`); `/particle`
//! decodes its options with the same function.

use mod_api::client::particles::PositionSource;
use sunrise_client::particles::{ParticleOptions, ParticleType};
use sunrise_core::nbt::{Compound, Tag};

/// A number tag as a float.
fn tag_f32(tag: &Tag) -> Option<f32> {
    Some(match tag {
        Tag::Byte(v) => f32::from(*v),
        Tag::Short(v) => f32::from(*v),
        Tag::Int(v) => *v as f32,
        Tag::Long(v) => *v as f32,
        Tag::Float(v) => f32::from_bits(*v),
        Tag::Double(v) => f64::from_bits(*v) as f32,
        _ => return None,
    })
}
/// A number tag as `Codec.INT` reads it (`Number.intValue`).
fn tag_i32(tag: &Tag) -> Option<i32> {
    Some(match tag {
        Tag::Byte(v) => i32::from(*v),
        Tag::Short(v) => i32::from(*v),
        Tag::Int(v) => *v,
        Tag::Long(v) => *v as i32,
        Tag::Float(v) => f32::from_bits(*v) as i32,
        Tag::Double(v) => f64::from_bits(*v) as i32,
        _ => return None,
    })
}
/// `ExtraCodecs.RGB_COLOR_CODEC` (`floats` 3) or `ARGB_COLOR_CODEC` (4): an
/// int, or a list of exactly that many floats packed through
/// `ARGB.colorFromFloat`; the channels as `ARGB` reads them back. The RGB
/// codec keeps no alpha (opaque).
fn tag_color(tag: Option<&Tag>, floats: usize) -> Option<[f32; 4]> {
    let packed = match tag? {
        Tag::List(l) if l.len() == floats => {
            let mut c = [1.0f32; 4];
            for (slot, value) in c.iter_mut().zip(l) {
                *slot = tag_f32(value)?;
            }
            // ARGB.as8BitChannel, then ARGB.color's byte masks.
            let byte = |f: f32| ((f * 255.0).floor() as i32) & 255;
            byte(c[3]) << 24 | byte(c[0]) << 16 | byte(c[1]) << 8 | byte(c[2])
        }
        Tag::List(_) => return None,
        other => tag_i32(other)?,
    };
    let channel = |shift: i32| ((packed >> shift) & 255) as f32 / 255.0;
    let alpha = if floats == 4 { channel(24) } else { 1.0 };
    Some([channel(16), channel(8), channel(0), alpha])
}
fn tag_rgb(tag: Option<&Tag>) -> Option<[f32; 3]> {
    let [r, g, b, _] = tag_color(tag, 3)?;
    Some([r, g, b])
}
/// `BlockPos.CODEC` (`Codec.INT_STREAM`, three ints).
fn tag_block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    match tag? {
        Tag::List(l) if l.len() == 3 => {
            let mut pos = [0; 3];
            for (slot, value) in pos.iter_mut().zip(l) {
                *slot = tag_i32(value)?;
            }
            Some(pos)
        }
        _ => None,
    }
}
/// The particle a saved `custom_particle` names (`ParticleTypes.CODEC`).
pub fn custom_particle_options(tag: &Tag) -> Option<ParticleOptions> {
    let Tag::Compound(map) = tag else {
        return None;
    };
    let Some(Tag::String(name)) = map.get(&"type".into()) else {
        return None;
    };
    particle_options(ParticleType::from_name(&name.to_string_lossy())?, map)
}
/// `ParticleTypes.CODEC`'s payload for `ty`: the type's own options codec. A
/// type that carries a block or an item, or one that does not decode, gives
/// `None`.
pub fn particle_options(ty: ParticleType, map: &Compound) -> Option<ParticleOptions> {
    let field = |key: &str| map.get(&key.into());
    let float = |key: &str, default: f32| field(key).and_then(tag_f32).unwrap_or(default);
    // ScalableParticleOptionsBase.SCALE: required, within [0.01, 4].
    let scale = || {
        field("scale")
            .and_then(tag_f32)
            .filter(|s| (0.01..=4.0).contains(s))
    };
    Some(match ty {
        ParticleType::Block
        | ParticleType::BlockCrumble
        | ParticleType::BlockMarker
        | ParticleType::FallingDust
        | ParticleType::DustPillar
        | ParticleType::Item
        // TrailParticleOption's `target` is a literal point, but a LAN host
        // sends no trail (`lan_particles::hub_particle`), so drawing it here
        // would draw it for the host only.
        | ParticleType::Trail => return None,
        // GeyserParticleOptions / GeyserBaseParticleOptions: `water_blocks` is
        // a positive int, `burst_impulse_base` a float.
        ParticleType::Geyser | ParticleType::GeyserPlume => {
            ParticleOptions::geyser(ty, tag_i32(field("water_blocks")?)?)
        }
        ParticleType::GeyserBase | ParticleType::GeyserPoof => ParticleOptions::geyser_base(
            ty,
            tag_i32(field("water_blocks")?)?,
            field("burst_impulse_base").and_then(tag_f32)?,
        ),
        // DustParticleOptions: color and scale, both required.
        ParticleType::Dust => ParticleOptions::dust(tag_rgb(field("color"))?, scale()?),
        ParticleType::DustColorTransition => ParticleOptions::dust_transition(
            tag_rgb(field("from_color"))?,
            tag_rgb(field("to_color"))?,
            scale()?,
        ),
        // ColorParticleOption: a required ARGB colour.
        ParticleType::EntityEffect | ParticleType::TintedLeaves | ParticleType::Flash => {
            ParticleOptions::colored(ty, tag_color(field("color"), 4)?)
        }
        // SpellParticleOption: color (default -1, white) and power.
        ParticleType::Effect | ParticleType::InstantEffect => {
            let [r, g, b] = match field("color") {
                Some(color) => tag_rgb(Some(color))?,
                None => [1.0; 3],
            };
            ParticleOptions {
                scale: float("power", 1.0),
                ..ParticleOptions::colored(ty, [r, g, b, 1.0])
            }
        }
        // PowerParticleOption.
        ParticleType::DragonBreath => ParticleOptions {
            scale: float("power", 1.0),
            ..ParticleOptions::simple(ty)
        },
        // SculkChargeParticleOptions.roll and ShriekParticleOption.delay are required.
        ParticleType::SculkCharge => {
            ParticleOptions::sculk_charge(field("roll").and_then(tag_f32)?)
        }
        ParticleType::Shriek => ParticleOptions::shriek(field("delay").and_then(tag_i32)?),
        // VibrationParticleOption: `destination` and `arrival_in_ticks`.
        // `SAFE_POSITION_SOURCE_CODEC` allows a block position source only,
        // whose centre is the point the signal flies to.
        ParticleType::Vibration => {
            let Tag::Compound(dest) = field("destination")? else {
                return None;
            };
            let Some(Tag::String(kind)) = dest.get(&"type".into()) else {
                return None;
            };
            if kind.to_string_lossy() != "minecraft:block" {
                return None;
            }
            let pos = tag_block_pos(dest.get(&"pos".into()))?;
            ParticleOptions::vibration(
                PositionSource::Block(pos),
                pos.map(|c| f64::from(c) + 0.5),
                tag_i32(field("arrival_in_ticks")?)?,
            )
        }
        _ => ParticleOptions::simple(ty),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flt(v: f32) -> Tag {
        Tag::Float(v.to_bits())
    }

    /// The options codecs' rules: `ColorParticleOption` keeps the colour's
    /// alpha (an int's top byte, the fourth float), float lists have a fixed
    /// length and are packed to 8-bit channels, dust needs a scale in
    /// [0.01, 4], and a spell's colour defaults to white. What fails to decode
    /// gives no custom particle.
    #[test]
    fn custom_particle_follows_the_option_codecs() {
        let decode = |fields: Vec<(&str, Tag)>| custom_particle_options(&Tag::compound(fields));
        let s = |v: &str| Tag::String(v.into());
        // 0x80FF0000: alpha 128, red.
        let tinted = decode(vec![
            ("type", s("entity_effect")),
            ("color", Tag::Int(0x80FF_0000u32 as i32)),
        ])
        .unwrap();
        assert_eq!(tinted.color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        // [r, g, b, a] = [0, 1, 0, 0.5]: floor(0.5 * 255) = 127.
        let listed = decode(vec![
            ("type", s("entity_effect")),
            (
                "color",
                Tag::List(vec![flt(0.0), flt(1.0), flt(0.0), flt(0.5)]),
            ),
        ])
        .unwrap();
        assert_eq!(listed.color, [0.0, 1.0, 0.0, 127.0 / 255.0]);
        let three = vec![flt(1.0), flt(0.0), flt(0.0)];
        assert!(
            decode(vec![
                ("type", s("entity_effect")),
                ("color", Tag::List(three.clone()))
            ])
            .is_none()
        );
        // RGB_COLOR_CODEC: 0.3 * 255 = 76.5, floored to 76.
        let dust = decode(vec![
            ("type", s("dust")),
            ("color", Tag::List(vec![flt(0.3), flt(0.0), flt(1.0)])),
            ("scale", flt(4.0)),
        ])
        .unwrap();
        assert_eq!(dust.color[..3], [76.0 / 255.0, 0.0, 1.0]);
        assert!(
            decode(vec![
                ("type", s("dust")),
                ("color", Tag::List(three.clone()))
            ])
            .is_none()
        );
        assert!(
            decode(vec![
                ("type", s("dust")),
                ("color", Tag::List(three.clone())),
                ("scale", flt(4.5))
            ])
            .is_none()
        );
        assert!(
            decode(vec![
                ("type", s("dust")),
                ("color", Tag::List(vec![flt(1.0); 4])),
                ("scale", flt(1.0))
            ])
            .is_none()
        );
        let spell = decode(vec![("type", s("effect"))]).unwrap();
        assert_eq!(spell.color[..3], [1.0, 1.0, 1.0]);
        assert!(decode(vec![("type", s("sculk_charge"))]).is_none());
        assert!(decode(vec![("type", s("shriek"))]).is_none());
        assert_eq!(
            decode(vec![("type", s("shriek")), ("delay", Tag::Int(7))])
                .unwrap()
                .ty,
            ParticleType::Shriek
        );
    }
}
