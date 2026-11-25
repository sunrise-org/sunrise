//! Data-driven mob variant and sound-variant registries (`VariantUtils`,
//! `SpawnPrioritySelectors`, `WolfVariant`, `WolfSoundVariant`, `CatVariant`,
//! `CatSoundVariant`, and the later families as they move onto `REGISTRY_KINDS`).
//!
//! Each family's frozen world registry is a [`VariantTable<T>`] whose entries
//! are sorted in vanilla's `ResourceManagerRegistryLoadTask` file order
//! (`(format!("{path}.json"), namespace)`), matching `lan::data::SyncData`'s
//! network ids. On a client, [`VariantTable::from_synced`] keeps the server's
//! exact sync order.
use super::finalize::FinalizeLevel;
use crate::lan::data::RegistryRef;
use serde_json::Value;
use std::{fs, path::Path};
use sunrise_core::{
    identifier::Identifier,
    nbt::{Compound, JavaString, Tag},
    random::RandomSource,
};
use sunrise_protocol::registry::Dynamic;

/// `SpawnCondition` (`minecraft:biome`, `minecraft:structure`, `minecraft:moon_brightness`).
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnCondition {
    /// `BiomeCheck`: canonical `"ns:id"` or `"#ns:tag"` entries.
    Biome(Vec<String>),
    /// `StructureCheck`: canonical `"ns:id"` or `"#ns:tag"` entries.
    Structure(Vec<String>),
    /// `MoonBrightnessCheck`: `MinMaxBounds.Doubles` over `MOON_BRIGHTNESS`.
    MoonBrightness { min: Option<f64>, max: Option<f64> },
}

/// `PriorityProvider.Selector<SpawnContext, SpawnCondition>`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnSelector {
    pub condition: Option<SpawnCondition>,
    pub priority: i32,
}

/// `PriorityProvider<SpawnContext, SpawnCondition>`.
pub trait HasSpawnConditions {
    fn spawn_conditions(&self) -> &[SpawnSelector];
}

/// Decode `SpawnPrioritySelectors.CODEC` (`spawn_conditions`).
pub fn decode_spawn_conditions(value: &Value) -> Result<Vec<SpawnSelector>, String> {
    let list = value
        .as_array()
        .ok_or("spawn_conditions: a list is required")?;
    let mut selectors = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("spawn_conditions[{index}]: not a map"))?;
        let priority = obj
            .get("priority")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| format!("spawn_conditions[{index}].priority: an int is required"))?;
        let condition = match obj.get("condition") {
            None => None,
            Some(c) => Some(decode_spawn_condition(
                &format!("spawn_conditions[{index}].condition"),
                c,
            )?),
        };
        selectors.push(SpawnSelector {
            condition,
            priority,
        });
    }
    Ok(selectors)
}

fn decode_spawn_condition(field: &str, value: &Value) -> Result<SpawnCondition, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{field}: not a map"))?;
    let raw_type = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field}.type: a string is required"))?;
    let kind = Identifier::parse(raw_type)
        .map_err(|e| format!("{field}.type: invalid identifier {raw_type:?}: {e}"))?;
    match kind.as_str() {
        "minecraft:biome" => {
            let biomes = obj
                .get("biomes")
                .ok_or_else(|| format!("{field}.biomes is required"))?;
            Ok(SpawnCondition::Biome(decode_holder_set(
                &format!("{field}.biomes"),
                biomes,
            )?))
        }
        "minecraft:structure" => {
            let structures = obj
                .get("structures")
                .ok_or_else(|| format!("{field}.structures is required"))?;
            Ok(SpawnCondition::Structure(decode_holder_set(
                &format!("{field}.structures"),
                structures,
            )?))
        }
        "minecraft:moon_brightness" => {
            let range = obj
                .get("range")
                .ok_or_else(|| format!("{field}.range is required"))?;
            let (min, max) = decode_double_bounds(&format!("{field}.range"), range)?;
            Ok(SpawnCondition::MoonBrightness { min, max })
        }
        other => Err(format!("{field}.type: unknown spawn condition {other:?}")),
    }
}

/// `RegistryCodecs.holderSet`: `"#ns:tag"`, `"ns:id"`, or `["ns:id", ...]`.
fn decode_holder_set(field: &str, value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(raw) => {
            if let Some(tag) = raw.strip_prefix('#') {
                let id = Identifier::parse(tag)
                    .map_err(|e| format!("{field}: invalid tag {raw:?}: {e}"))?;
                Ok(vec![format!("#{}", id.as_str())])
            } else {
                let id = Identifier::parse(raw)
                    .map_err(|e| format!("{field}: invalid identifier {raw:?}: {e}"))?;
                Ok(vec![id.as_str().to_owned()])
            }
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let raw = item
                    .as_str()
                    .ok_or_else(|| format!("{field}[{i}]: a string is required"))?;
                let id = Identifier::parse(raw)
                    .map_err(|e| format!("{field}[{i}]: invalid identifier {raw:?}: {e}"))?;
                Ok(id.as_str().to_owned())
            })
            .collect(),
        _ => Err(format!("{field}: a string or list of strings is required")),
    }
}

/// `MinMaxBounds.Doubles.CODEC`: a finite number or `{ min?, max? }`.
fn decode_double_bounds(field: &str, value: &Value) -> Result<(Option<f64>, Option<f64>), String> {
    if let Some(exact) = value.as_f64() {
        if !exact.is_finite() {
            return Err(format!("{field}: must be finite"));
        }
        return Ok((Some(exact), Some(exact)));
    }
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{field}: a number or map is required"))?;
    let read = |key: &str| -> Result<Option<f64>, String> {
        match obj.get(key) {
            None => Ok(None),
            Some(v) => {
                let n = v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("{field}.{key}: a finite number is required"))?;
                Ok(Some(n))
            }
        }
    };
    let min = read("min")?;
    let max = read("max")?;
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!(
            "{field}: min ({lo}) cannot be greater than max ({hi})"
        ));
    }
    Ok((min, max))
}

/// One frozen world registry of mob variants or sound variants, indexed by
/// network id (`RegistryRef<T>`).
#[derive(Clone, Debug)]
pub struct VariantTable<T> {
    entries: Vec<(String, T)>,
}

impl<T> Default for VariantTable<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> VariantTable<T> {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a server-side world table in vanilla's `ResourceManagerRegistryLoadTask`
    /// file order (`(format!("{path}.json"), namespace)`), matching `SyncData`.
    pub fn from_entries(mut entries: Vec<((String, String), T)>) -> Self {
        entries.sort_by_cached_key(|((ns, path), _)| (format!("{path}.json"), ns.clone()));
        Self {
            entries: entries
                .into_iter()
                .map(|((ns, path), value)| (format!("{ns}:{path}"), value))
                .collect(),
        }
    }

    /// Build a client-side table preserving the server's exact sync order.
    pub fn from_synced(entries: Vec<(String, T)>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, r: RegistryRef<T>) -> Option<&T> {
        self.entries.get(r.network_id() as usize).map(|(_, v)| v)
    }

    pub fn id(&self, r: RegistryRef<T>) -> Option<&str> {
        self.entries
            .get(r.network_id() as usize)
            .map(|(id, _)| id.as_str())
    }

    pub fn entry(&self, r: RegistryRef<T>) -> Option<(&str, &T)> {
        self.entries
            .get(r.network_id() as usize)
            .map(|(id, v)| (id.as_str(), v))
    }

    /// Look up an entry by canonical or default-namespace identifier.
    pub fn lookup(&self, id: &str) -> Option<RegistryRef<T>> {
        let canonical = Identifier::parse(id).ok()?;
        let target = canonical.as_str();
        self.entries
            .iter()
            .position(|(name, _)| name == target)
            .map(|i| RegistryRef::new(i as u32))
    }

    /// `VariantUtils.getDefaultOrAny`: `default_id` if present, else the first entry.
    pub fn default_or_any(&self, default_id: &str) -> Option<RegistryRef<T>> {
        self.lookup(default_id)
            .or_else(|| (!self.entries.is_empty()).then(|| RegistryRef::new(0)))
    }

    /// `Registry.getRandom`: any entry of the table.
    pub fn random(&self, random: &mut impl RandomSource) -> Option<RegistryRef<T>> {
        if self.entries.is_empty() {
            return None;
        }
        let i = random
            .next_int_bound(self.entries.len() as i32)
            .unwrap_or(0) as u32;
        Some(RegistryRef::new(i))
    }

    pub fn iter(&self) -> impl Iterator<Item = (RegistryRef<T>, &str, &T)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, (id, v))| (RegistryRef::new(i as u32), id.as_str(), v))
    }
}

impl<T: HasSpawnConditions> VariantTable<T> {
    /// `VariantUtils.selectVariantToSpawn` / `PriorityProvider.pick`: the entries
    /// of the highest priority whose condition holds, one of them at random.
    pub fn select<L: FinalizeLevel + ?Sized>(
        &self,
        level: &L,
        pos: [i32; 3],
        random: &mut impl RandomSource,
    ) -> Option<RegistryRef<T>> {
        let mut unpacked: Vec<(RegistryRef<T>, Option<&SpawnCondition>, i32)> = Vec::new();
        for (i, (_, entry)) in self.entries.iter().enumerate() {
            let r = RegistryRef::new(i as u32);
            for selector in entry.spawn_conditions() {
                unpacked.push((r, selector.condition.as_ref(), selector.priority));
            }
        }
        unpacked.sort_by_key(|(_, _, priority)| std::cmp::Reverse(*priority));
        let mut highest = i32::MIN;
        let mut selected = Vec::new();
        for (r, condition, priority) in unpacked {
            if priority < highest {
                continue;
            }
            let ok = match condition {
                None => true,
                Some(SpawnCondition::Biome(list)) => {
                    list.iter().any(|b| match b.strip_prefix('#') {
                        Some(tag) => level.biome_tag(tag, pos),
                        None => level.biome(pos) == *b,
                    })
                }
                Some(SpawnCondition::Structure(list)) => {
                    let here = level.structures_with_piece_at(pos);
                    list.iter().any(|s| match s.strip_prefix('#') {
                        Some(tag) => here.iter().any(|h| level.structure_tag(tag, h)),
                        None => here.iter().any(|h| h == s),
                    })
                }
                Some(SpawnCondition::MoonBrightness { min, max }) => {
                    let b = f64::from(super::spawning::MOON_BRIGHTNESS[level.moon_phase(pos) & 7]);
                    min.is_none_or(|m| b >= m) && max.is_none_or(|m| b <= m)
                }
            };
            if ok {
                highest = priority;
                selected.push(r);
            }
        }
        if selected.is_empty() {
            return None;
        }
        let pick = random.next_int_bound(selected.len() as i32).unwrap_or(0) as usize;
        Some(selected[pick])
    }
}

/// `WolfVariant.AssetInfo`: wild, tame and angry texture asset ids (canonical `ns:path`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfAssetInfo {
    pub wild: String,
    pub tame: String,
    pub angry: String,
}

impl WolfAssetInfo {
    fn missing() -> Self {
        Self {
            wild: String::new(),
            tame: String::new(),
            angry: String::new(),
        }
    }

    fn decode(field: &str, value: &Value) -> Result<Self, String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let asset = |key: &str| -> Result<String, String> {
            let raw = obj
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field}.{key}: a string is required"))?;
            let id = Identifier::parse(raw)
                .map_err(|e| format!("{field}.{key}: invalid identifier {raw:?}: {e}"))?;
            Ok(id.as_str().to_owned())
        };
        Ok(Self {
            wild: asset("wild")?,
            tame: asset("tame")?,
            angry: asset("angry")?,
        })
    }

    fn to_nbt(&self) -> Tag {
        Tag::compound([
            ("wild", Tag::String(JavaString::from(self.wild.as_str()))),
            ("tame", Tag::String(JavaString::from(self.tame.as_str()))),
            ("angry", Tag::String(JavaString::from(self.angry.as_str()))),
        ])
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            wild: read("wild")?,
            tame: read("tame")?,
            angry: read("angry")?,
        })
    }
}

/// `WolfVariant`: adult and baby texture asset sets plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct WolfVariant {
    pub assets: WolfAssetInfo,
    pub baby_assets: WolfAssetInfo,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for WolfVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl WolfVariant {
    fn missing() -> Self {
        Self {
            assets: WolfAssetInfo::missing(),
            baby_assets: WolfAssetInfo::missing(),
            spawn_conditions: Vec::new(),
        }
    }

    /// `WolfVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let assets =
            WolfAssetInfo::decode("assets", obj.get("assets").ok_or("assets is required")?)?;
        let baby_assets = WolfAssetInfo::decode(
            "baby_assets",
            obj.get("baby_assets").ok_or("baby_assets is required")?,
        )?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            assets,
            baby_assets,
            spawn_conditions,
        })
    }

    /// `WolfVariant.NETWORK_CODEC`: `assets` and `baby_assets` only.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            ("assets", self.assets.to_nbt()),
            ("baby_assets", self.baby_assets.to_nbt()),
        ])
    }

    /// Decode a client's synced `WolfVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        let assets = WolfAssetInfo::decode_nbt(tag.get("assets")?)?;
        let baby_assets = WolfAssetInfo::decode_nbt(tag.get("baby_assets")?)?;
        Some(Self {
            assets,
            baby_assets,
            spawn_conditions: Vec::new(),
        })
    }

    /// `Wolf.getTexture`: the atlas sprite key for this variant's age and mood.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self, baby: bool, tame: bool, angry: bool) -> &str {
        let info = if baby {
            &self.baby_assets
        } else {
            &self.assets
        };
        let asset = if tame {
            &info.tame
        } else if angry {
            &info.angry
        } else {
            &info.wild
        };
        asset.strip_prefix("minecraft:").unwrap_or(asset)
    }
}

/// `WolfSoundVariant.WolfSoundSet`: the seven sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfSoundSet {
    pub ambient: String,
    pub death: String,
    pub growl: String,
    pub hurt: String,
    pub pant: String,
    pub whine: String,
    pub step: String,
}

impl WolfSoundSet {
    fn missing() -> Self {
        Self {
            ambient: String::new(),
            death: String::new(),
            growl: String::new(),
            hurt: String::new(),
            pant: String::new(),
            whine: String::new(),
            step: String::new(),
        }
    }

    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            death: read("death_sound")?,
            growl: read("growl_sound")?,
            hurt: read("hurt_sound")?,
            pant: read("pant_sound")?,
            whine: read("whine_sound")?,
            step: read("step_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            Some(Tag::Compound(_)) => match tag.get(key)?.get("sound_id") {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        };
        Some(Self {
            ambient: read("ambient_sound")?,
            death: read("death_sound")?,
            growl: read("growl_sound")?,
            hurt: read("hurt_sound")?,
            pant: read("pant_sound")?,
            whine: read("whine_sound")?,
            step: read("step_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "death" => &self.death,
            "growl" => &self.growl,
            "hurt" => &self.hurt,
            "pant" => &self.pant,
            "whine" => &self.whine,
            "step" => &self.step,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `WolfSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfSoundVariant {
    pub adult_sounds: WolfSoundSet,
    pub baby_sounds: WolfSoundSet,
}

impl WolfSoundVariant {
    fn missing() -> Self {
        Self {
            adult_sounds: WolfSoundSet::missing(),
            baby_sounds: WolfSoundSet::missing(),
        }
    }

    /// `WolfSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = WolfSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = WolfSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `WolfSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: WolfSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: WolfSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Wolf.getSoundSet`: the sound id for `kind` (`ambient`, `death`, `growl`,
    /// `hurt`, `pant`, `whine`, `step`) in the adult or baby set.
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

/// `CatVariant`: adult and baby texture asset ids plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct CatVariant {
    pub asset_id: String,
    pub baby_asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for CatVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl CatVariant {
    fn missing() -> Self {
        Self {
            asset_id: String::new(),
            baby_asset_id: String::new(),
            spawn_conditions: Vec::new(),
        }
    }

    /// `CatVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let asset = |key: &str| -> Result<String, String> {
            let raw = obj
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key}: a string is required"))?;
            let id = Identifier::parse(raw)
                .map_err(|e| format!("{key}: invalid identifier {raw:?}: {e}"))?;
            Ok(id.as_str().to_owned())
        };
        let asset_id = asset("asset_id")?;
        let baby_asset_id = asset("baby_asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            asset_id,
            baby_asset_id,
            spawn_conditions,
        })
    }

    /// `CatVariant.NETWORK_CODEC`: `asset_id` and `baby_asset_id` only.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            (
                "asset_id",
                Tag::String(JavaString::from(self.asset_id.as_str())),
            ),
            (
                "baby_asset_id",
                Tag::String(JavaString::from(self.baby_asset_id.as_str())),
            ),
        ])
    }

    /// Decode a client's synced `CatVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            asset_id: read("asset_id")?,
            baby_asset_id: read("baby_asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `CatVariant.assetInfo`: the atlas sprite key for this variant's age.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self, baby: bool) -> &str {
        let asset = if baby {
            &self.baby_asset_id
        } else {
            &self.asset_id
        };
        asset.strip_prefix("minecraft:").unwrap_or(asset)
    }
}

/// `CatSoundVariant.CatSoundSet`: the nine sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatSoundSet {
    pub ambient: String,
    pub stray_ambient: String,
    pub hiss: String,
    pub hurt: String,
    pub death: String,
    pub eat: String,
    pub beg_for_food: String,
    pub purr: String,
    pub purreow: String,
}

impl CatSoundSet {
    fn missing() -> Self {
        Self {
            ambient: String::new(),
            stray_ambient: String::new(),
            hiss: String::new(),
            hurt: String::new(),
            death: String::new(),
            eat: String::new(),
            beg_for_food: String::new(),
            purr: String::new(),
            purreow: String::new(),
        }
    }

    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            stray_ambient: read("stray_ambient_sound")?,
            hiss: read("hiss_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            eat: read("eat_sound")?,
            beg_for_food: read("beg_for_food_sound")?,
            purr: read("purr_sound")?,
            purreow: read("purreow_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            Some(Tag::Compound(_)) => match tag.get(key)?.get("sound_id") {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        };
        Some(Self {
            ambient: read("ambient_sound")?,
            stray_ambient: read("stray_ambient_sound")?,
            hiss: read("hiss_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            eat: read("eat_sound")?,
            beg_for_food: read("beg_for_food_sound")?,
            purr: read("purr_sound")?,
            purreow: read("purreow_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "stray_ambient" => &self.stray_ambient,
            "hiss" => &self.hiss,
            "hurt" => &self.hurt,
            "death" => &self.death,
            "eat" => &self.eat,
            "beg_for_food" => &self.beg_for_food,
            "purr" => &self.purr,
            "purreow" => &self.purreow,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `CatSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatSoundVariant {
    pub adult_sounds: CatSoundSet,
    pub baby_sounds: CatSoundSet,
}

impl CatSoundVariant {
    fn missing() -> Self {
        Self {
            adult_sounds: CatSoundSet::missing(),
            baby_sounds: CatSoundSet::missing(),
        }
    }

    /// `CatSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = CatSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = CatSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `CatSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: CatSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: CatSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Cat.getSoundSet`: the sound id for `kind` (`ambient`, `stray_ambient`,
    /// `hiss`, `hurt`, `death`, `eat`, `beg_for_food`, `purr`, `purreow`) in
    /// the adult or baby set.
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

/// The frozen world variant tables converted onto `REGISTRY_KINDS`.
#[derive(Clone, Debug, Default)]
pub struct VariantTables {
    pub wolf: VariantTable<WolfVariant>,
    pub wolf_sound: VariantTable<WolfSoundVariant>,
    pub cat: VariantTable<CatVariant>,
    pub cat_sound: VariantTable<CatSoundVariant>,
}

impl VariantTables {
    /// Whether no variant table has been populated yet.
    pub fn is_empty(&self) -> bool {
        self.wolf.is_empty() && self.cat.is_empty()
    }

    /// Load the prepared vanilla tables (`data/minecraft/<registry>/`) for worlds
    /// without packs, worldgen helpers and standalone probes.
    pub fn load_prepared(root: &Path) -> Self {
        let data = root.join("data/minecraft");
        let wolf = load_prepared_table(&data.join("wolf_variant"), WolfVariant::decode);
        let wolf_sound =
            load_prepared_table(&data.join("wolf_sound_variant"), decode_prepared_wolf_sound);
        let cat = load_prepared_table(&data.join("cat_variant"), CatVariant::decode);
        let cat_sound =
            load_prepared_table(&data.join("cat_sound_variant"), decode_prepared_cat_sound);
        Self {
            wolf,
            wolf_sound,
            cat,
            cat_sound,
        }
    }

    /// Build a client's variant tables from the server's synchronized `Dynamic`
    /// registries, keeping the server's exact network id order and resolving
    /// known-pack omitted entries from `fallback`.
    pub fn from_synced(registries: &Dynamic, fallback: &VariantTables) -> Self {
        let wolf = synced_table(
            registries,
            "minecraft:wolf_variant",
            WolfVariant::decode_network_nbt,
            &fallback.wolf,
            WolfVariant::missing,
        );
        let wolf_sound = synced_table(
            registries,
            "minecraft:wolf_sound_variant",
            WolfSoundVariant::decode_network_nbt,
            &fallback.wolf_sound,
            WolfSoundVariant::missing,
        );
        let cat = synced_table(
            registries,
            "minecraft:cat_variant",
            CatVariant::decode_network_nbt,
            &fallback.cat,
            CatVariant::missing,
        );
        let cat_sound = synced_table(
            registries,
            "minecraft:cat_sound_variant",
            CatSoundVariant::decode_network_nbt,
            &fallback.cat_sound,
            CatSoundVariant::missing,
        );
        Self {
            wolf,
            wolf_sound,
            cat,
            cat_sound,
        }
    }
}

fn synced_table<T: Clone>(
    registries: &Dynamic,
    key: &str,
    decode: fn(&Tag) -> Option<T>,
    fallback: &VariantTable<T>,
    missing: fn() -> T,
) -> VariantTable<T> {
    registries
        .registries
        .get(key)
        .map(|entries| {
            VariantTable::from_synced(
                entries
                    .iter()
                    .map(|(id, data)| {
                        let entry = data
                            .as_ref()
                            .and_then(decode)
                            .or_else(|| fallback.lookup(id).and_then(|r| fallback.get(r)).cloned())
                            .unwrap_or_else(missing);
                        (id.clone(), entry)
                    })
                    .collect(),
            )
        })
        .unwrap_or_default()
}

fn load_prepared_table<T>(
    dir: &Path,
    decode: impl Fn(&Value) -> Result<T, String>,
) -> VariantTable<T> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_suffix(".json"))
            .map(str::to_owned)
        else {
            continue;
        };
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Ok(decoded) = decode(&value) {
            entries.push((("minecraft".to_owned(), name), decoded));
        }
    }
    VariantTable::from_entries(entries)
}

fn decode_prepared_sound_holder(field: &str, raw: &Value) -> Result<(String, Tag), String> {
    match raw {
        Value::String(s) => {
            let id = Identifier::parse(s)
                .map_err(|e| format!("{field}: invalid sound identifier {s:?}: {e}"))?;
            Ok((
                id.as_str().to_owned(),
                Tag::String(JavaString::from(id.as_str())),
            ))
        }
        Value::Object(map) => {
            let raw_id = map
                .get("sound_id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field}.sound_id: a string is required"))?;
            let id = Identifier::parse(raw_id)
                .map_err(|e| format!("{field}.sound_id: invalid identifier {raw_id:?}: {e}"))?;
            Ok((
                id.as_str().to_owned(),
                Tag::String(JavaString::from(id.as_str())),
            ))
        }
        _ => Err(format!("{field}: a sound event is required")),
    }
}

fn decode_prepared_wolf_sound(value: &Value) -> Result<WolfSoundVariant, String> {
    let (decoded, _) = WolfSoundVariant::decode_with(value, decode_prepared_sound_holder)?;
    Ok(decoded)
}

fn decode_prepared_cat_sound(value: &Value) -> Result<CatSoundVariant, String> {
    let (decoded, _) = CatSoundVariant::decode_with(value, decode_prepared_sound_holder)?;
    Ok(decoded)
}

#[cfg(test)]
#[path = "wolf_variant_tests.rs"]
mod wolf_variant_tests;

#[cfg(test)]
#[path = "cat_variant_tests.rs"]
mod cat_variant_tests;
