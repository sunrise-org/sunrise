use super::{
    DisplayContext, Result, boolean, component_predicate, component_type, float, foreign, get,
    ident, is_range_property, is_select_property, is_tint_source, non_negative, object, opaque,
    required, rgb, string,
};
use crate::json::error as err;
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Tint sources
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum TintSource {
    Constant(i32),
    CustomModelData {
        index: i32,
        default: i32,
    },
    Dye(i32),
    Grass {
        temperature: f32,
        downfall: f32,
    },
    Firework(i32),
    Potion(i32),
    Team(i32),
    /// A source a native mod registered (`register_tint_sources`), read through
    /// [`ItemEnv::item_tint_source`] with its entry in the model's `tints`.
    Mod {
        id: String,
        config: Arc<sunrise_core::nbt::Tag>,
    },
}
impl TintSource {
    pub(super) fn decode(v: &Value) -> Result<Self> {
        let m = object(v)?;
        let default = || rgb(required(m, "default")?);
        let unit = |v: &Value| -> Result<f32> {
            let f = float(v)?;
            if (0.0..=1.0).contains(&f) {
                Ok(f)
            } else {
                Err(err("value must be within range [0;1]"))
            }
        };
        Ok(match ident(required(m, "type")?)?.as_str() {
            "minecraft:constant" => Self::Constant(opaque(rgb(required(m, "value")?)?)),
            "minecraft:custom_model_data" => Self::CustomModelData {
                index: get(m, "index").map_or(Ok(0), non_negative)?,
                default: default()?,
            },
            "minecraft:dye" => Self::Dye(default()?),
            "minecraft:grass" => Self::Grass {
                temperature: unit(required(m, "temperature")?)?,
                downfall: unit(required(m, "downfall")?)?,
            },
            "minecraft:firework" => Self::Firework(default()?),
            "minecraft:potion" => Self::Potion(default()?),
            "minecraft:team" => Self::Team(default()?),
            other if is_tint_source(other) => Self::Mod {
                id: other.to_owned(),
                config: Arc::new(
                    sunrise_core::json_nbt::json_to_tag(v)
                        .map_err(|_| err("unknown item tint source"))?,
                ),
            },
            // A mod's own tint source, which its Java code computes: untinted here.
            other if foreign::is_foreign(other) => Self::Constant(-1),
            _ => return Err(err("unknown item tint source")),
        })
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ConditionProperty {
    Custom(String),
    CustomModelData(i32),
    UsingItem,
    Broken,
    Damaged,
    FishingRodCast,
    HasComponent {
        component: String,
        ignore_default: bool,
    },
    BundleHasSelectedItem,
    Selected,
    Carried,
    ExtendedView,
    KeybindDown(String),
    ViewEntity,
    /// A single `component` predicate; external predicate types use the host.
    Component(Value),
}
impl ConditionProperty {
    pub(super) fn decode(m: &Map<String, Value>) -> Result<Self> {
        Ok(match ident(required(m, "property")?)?.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:using_item" => Self::UsingItem,
            "minecraft:broken" => Self::Broken,
            "minecraft:damaged" => Self::Damaged,
            "minecraft:fishing_rod/cast" => Self::FishingRodCast,
            "minecraft:has_component" => Self::HasComponent {
                component: component_type(required(m, "component")?)?,
                ignore_default: get(m, "ignore_default").map_or(Ok(false), boolean)?,
            },
            "minecraft:bundle/has_selected_item" => Self::BundleHasSelectedItem,
            "minecraft:selected" => Self::Selected,
            "minecraft:carried" => Self::Carried,
            "minecraft:extended_view" => Self::ExtendedView,
            "minecraft:keybind_down" => Self::KeybindDown(string(required(m, "keybind")?)?),
            "minecraft:view_entity" => Self::ViewEntity,
            "minecraft:component" => Self::Component(component_predicate::decode(m)?),
            _ => return Err(err("unknown condition property")),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectProperty {
    /// A property a native mod registered (`register_select_properties`), read through
    /// [`ItemEnv::item_model_select_property`]; its `when` values are strings.
    Custom(String),
    CustomModelData(i32),
    MainHand,
    ChargeType,
    TrimMaterial,
    BlockState(String),
    DisplayContext,
    LocalTime {
        pattern: String,
        locale: String,
        time_zone: Option<String>,
    },
    ContextEntityType,
    ContextDimension,
    Component(String),
}
impl SelectProperty {
    pub(super) fn decode(m: &Map<String, Value>) -> Result<Self> {
        let property = ident(required(m, "property")?)?;
        if is_select_property(&property) {
            return Ok(Self::Custom(property));
        }
        Ok(match property.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:main_hand" => Self::MainHand,
            "minecraft:charge_type" => Self::ChargeType,
            "minecraft:trim_material" => Self::TrimMaterial,
            "minecraft:block_state" => {
                Self::BlockState(string(required(m, "block_state_property")?)?)
            }
            "minecraft:display_context" => Self::DisplayContext,
            "minecraft:local_time" => Self::LocalTime {
                pattern: string(required(m, "pattern")?)?,
                locale: get(m, "locale").map_or(Ok(String::new()), string)?,
                time_zone: get(m, "time_zone").map(string).transpose()?,
            },
            "minecraft:context_entity_type" => Self::ContextEntityType,
            "minecraft:context_dimension" => Self::ContextDimension,
            "minecraft:component" => Self::Component(component_type(required(m, "component")?)?),
            _ => return Err(err("unknown select property")),
        })
    }
    /// A `when` value in canonical form (identifiers normalised).
    pub(super) fn decode_value(&self, v: &Value) -> Result<Value> {
        Ok(match self {
            Self::Custom(_)
            | Self::CustomModelData(_)
            | Self::BlockState(_)
            | Self::LocalTime { .. } => Value::String(string(v)?),
            Self::MainHand => match string(v)?.as_str() {
                s @ ("left" | "right") => Value::String(s.to_owned()),
                _ => return Err(err("invalid arm")),
            },
            Self::ChargeType => match string(v)?.as_str() {
                s @ ("none" | "rocket" | "arrow") => Value::String(s.to_owned()),
                _ => return Err(err("invalid charge type")),
            },
            Self::TrimMaterial | Self::ContextEntityType | Self::ContextDimension => {
                Value::String(ident(v)?)
            }
            Self::DisplayContext => {
                let s = string(v)?;
                DisplayContext::by_name(&s).ok_or_else(|| err("invalid display context"))?;
                Value::String(s)
            }
            Self::Component(_) => v.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompassTarget {
    None,
    Lodestone,
    Spawn,
    Recovery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSource {
    Random,
    Daytime,
    MoonPhase,
}
/// A `NeedleDirectionHelper.Wobbler`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wobbler {
    pub rotation: f32,
    pub delta_rotation: f32,
    pub last_update_tick: i64,
    /// Without wobble the needle jumps straight to its target.
    pub target: f32,
}
impl Wobbler {
    pub fn should_update(&self, wobble: bool, tick: i64) -> bool {
        !wobble || self.last_update_tick != tick
    }
    pub fn update(&mut self, wobble: bool, factor: f32, tick: i64, target_rotation: f32) {
        if !wobble {
            self.target = target_rotation;
            return;
        }
        self.last_update_tick = tick;
        let temp = positive_modulo(target_rotation - self.rotation + 0.5, 1.0) - 0.5;
        self.delta_rotation += temp * 0.1;
        self.delta_rotation *= factor;
        self.rotation = positive_modulo(self.rotation + self.delta_rotation, 1.0);
    }
    pub fn value(&self, wobble: bool) -> f32 {
        if wobble { self.rotation } else { self.target }
    }
}
/// `Mth.positiveModulo(float, float)`.
pub fn positive_modulo(input: f32, modulus: f32) -> f32 {
    (input % modulus + modulus) % modulus
}
/// The mutable state a baked `compass` or `time` property keeps.
#[derive(Debug, Default)]
pub struct NeedleState(pub(super) Mutex<[Wobbler; 2]>);
impl Clone for NeedleState {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for NeedleState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NumericProperty {
    /// A property a native mod registered (`register_range_properties`), read through
    /// [`ItemEnv::item_model_range_property`].
    Custom(String),
    BundleFullness,
    Compass {
        wobble: bool,
        target: CompassTarget,
        state: Arc<NeedleState>,
    },
    Cooldown,
    Count {
        normalize: bool,
    },
    CrossbowPull,
    CustomModelData(i32),
    Damage {
        normalize: bool,
    },
    Time {
        wobble: bool,
        source: TimeSource,
        state: Arc<NeedleState>,
    },
    UseCycle(f32),
    UseDuration {
        remaining: bool,
    },
}
impl NumericProperty {
    pub(super) fn decode(m: &Map<String, Value>) -> Result<Self> {
        let normalize = || get(m, "normalize").map_or(Ok(true), boolean);
        let property = ident(required(m, "property")?)?;
        if is_range_property(&property) {
            return Ok(Self::Custom(property));
        }
        Ok(match property.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:bundle/fullness" => Self::BundleFullness,
            "minecraft:damage" => Self::Damage {
                normalize: normalize()?,
            },
            "minecraft:cooldown" => Self::Cooldown,
            "minecraft:time" => Self::Time {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                source: match string(required(m, "source")?)?.as_str() {
                    "random" => TimeSource::Random,
                    "daytime" => TimeSource::Daytime,
                    "moon_phase" => TimeSource::MoonPhase,
                    _ => return Err(err("invalid time source")),
                },
                state: Arc::default(),
            },
            "minecraft:compass" => Self::Compass {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                target: match string(required(m, "target")?)?.as_str() {
                    "none" => CompassTarget::None,
                    "lodestone" => CompassTarget::Lodestone,
                    "spawn" => CompassTarget::Spawn,
                    "recovery" => CompassTarget::Recovery,
                    _ => return Err(err("invalid compass target")),
                },
                state: Arc::default(),
            },
            "minecraft:crossbow/pull" => Self::CrossbowPull,
            "minecraft:use_cycle" => {
                let period = get(m, "period").map_or(Ok(1.0), float)?;
                if period.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                    return Err(err("value must be positive"));
                }
                Self::UseCycle(period)
            }
            "minecraft:use_duration" => Self::UseDuration {
                remaining: get(m, "remaining").map_or(Ok(false), boolean)?,
            },
            "minecraft:count" => Self::Count {
                normalize: normalize()?,
            },
            _ => return Err(err("unknown range dispatch property")),
        })
    }
}
