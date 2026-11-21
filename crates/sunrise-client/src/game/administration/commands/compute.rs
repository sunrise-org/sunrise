//! `/compute` (ComputeCommand): context number providers
//! (`context_int_provider`, `context_float_provider`) evaluated in a loot
//! context of the source, optionally with a block or a target entity.
use super::{Arg, Context, Game, R, Target, err};
use serde_json::Value;
use sunrise_client::{
    advancements::{self as adv, EntityView},
    command::CommandError,
    text::Text,
};
use sunrise_core::java_number::float_to_string;
use sunrise_core::random::RandomSource;

/// The loot context parameters of `LootContextSources`.
#[derive(Clone, Copy, Default)]
pub(in crate::game) struct ComputeContext {
    /// `BLOCK_STATE` (`compute block <pos>`).
    pub block: Option<[i32; 3]>,
    /// `TARGET_ENTITY` (`compute entity <target>`).
    pub target: Option<Target>,
    /// `INTERACTING_ENTITY` (the player a composter's `addLayer` names).
    pub interacting: Option<Target>,
    /// A context with no `THIS_ENTITY` (`LootContextParamSets.BLOCK_INTERACT`)
    /// and this `ORIGIN`, instead of the command source's entity and position.
    pub without_this: Option<[f64; 3]>,
}

/// The entities and origin of a compute context's conditions: `this` and the
/// origin come from the command source unless the context has no `this`.
fn context_entities<'a>(
    c: &ComputeContext,
    source_this: Option<&'a EntityView>,
    source_pos: [f64; 3],
    target: Option<&'a EntityView>,
    interacting: Option<&'a EntityView>,
) -> adv::Entities<'a> {
    adv::Entities {
        this: source_this.filter(|_| c.without_this.is_none()),
        origin: c.without_this.unwrap_or(source_pos),
        tool: None,
        block_pos: None,
        target,
        interacting,
    }
}

/// An ArithmeticException's message.
type Arith = String;

impl Game<'_> {
    /// A provider argument: its registry id (None when inline) and value.
    pub(in crate::game) fn provider_arg(
        &self,
        ctx: &Context,
        registry: &str,
    ) -> R<(Option<String>, Value)> {
        let raw = match ctx.get("provider") {
            Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
            _ => return Err(err("command.failed", &[])),
        };
        let first = raw.trim_start().chars().next().unwrap_or(' ');
        if first == '{' || first == '[' || first == '-' || first.is_ascii_digit() || first == '"' {
            let (tag, _) = sunrise_core::nbt::parse_snbt(&raw)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            return Ok((None, sunrise_client::scoreboard::tag_to_json(&tag)));
        }
        let id = if raw.contains(':') {
            raw
        } else {
            format!("minecraft:{raw}")
        };
        let value = self
            .provider_file(registry, &id)
            .ok_or_else(|| err("argument.resource_or_id.no_such_element", &[&id, &registry]))?;
        Ok((Some(id), value))
    }
    /// A registered provider's definition.
    fn provider_file(&self, registry: &str, id: &str) -> Option<Value> {
        let id = sunrise_client::command::args::normalized(id);
        match registry {
            "minecraft:predicate" => self.fns.store.predicates.get(id.as_ref()),
            _ => self.context_registry(registry)?.get(id.as_ref()),
        }
        .cloned()
    }
    /// The compute context the command's source literal selects.
    pub(in crate::game) fn compute_context(&mut self, ctx: &Context) -> R<ComputeContext> {
        let mut c = ComputeContext::default();
        if ctx.literals.iter().any(|l| l == "block") && ctx.get("computePos").is_some() {
            c.block = Some(self.block_pos_arg(ctx, "computePos")?);
        }
        if ctx.get("computeTarget").is_some() {
            let t = self.entities(ctx, "computeTarget", false)?;
            if t.len() > 1 {
                return Err(err("argument.entity.toomany", &[]));
            }
            c.target = t.first().copied();
        }
        Ok(c)
    }
    /// `ComputeCommand.computeAsInt` / `computeAsFloat`.
    pub(super) fn cmd_compute(&mut self, ctx: &Context) -> R<i32> {
        let integer = ctx.literals.iter().any(|l| l == "integer");
        let registry = if integer {
            "minecraft:context_int_provider"
        } else {
            "minecraft:context_float_provider"
        };
        let (id, provider) = self.provider_arg(ctx, registry)?;
        let c = self.compute_context(ctx)?;
        let invalid = |value: String| -> CommandError {
            match &id {
                Some(id) => CommandError::new(Text::translate(
                    "command.compute.result.named.invalid",
                    &[Text::literal(id.clone()), Text::literal(value)],
                )),
                None => CommandError::new(Text::translate(
                    "command.compute.result.unnamed.invalid",
                    &[Text::literal(value)],
                )),
            }
        };
        let exact = |result: i32| match &id {
            Some(id) => Text::translate(
                "command.compute.result.named.exact",
                &[Text::literal(id.clone()), Text::literal(result.to_string())],
            ),
            None => Text::translate(
                "command.compute.result.unnamed.exact",
                &[Text::literal(result.to_string())],
            ),
        };
        if integer {
            let result = self.provider_int(&provider, &c, 0).map_err(invalid)?;
            self.feedback(exact(result));
            return Ok(result);
        }
        let scale = match ctx.get("scale") {
            Some(Arg::Float(s)) => *s,
            _ => 1.0,
        };
        let original = self.provider_float(&provider, &c, 0).map_err(invalid)?;
        if !original.is_finite() {
            return Err(invalid(float_to_string(original)));
        }
        // Mth.floor(original * scale)
        let result = (original * scale).floor() as i32;
        if result as f32 == original {
            self.feedback(exact(result));
        } else {
            let original_text = Text::literal(float_to_string(original));
            let message = match &id {
                Some(id) => Text::translate(
                    "command.compute.result.named.rounded",
                    &[
                        Text::literal(id.clone()),
                        original_text,
                        Text::literal(result.to_string()),
                    ],
                ),
                None => Text::translate(
                    "command.compute.result.unnamed.rounded",
                    &[original_text, Text::literal(result.to_string())],
                ),
            };
            self.feedback(message);
        }
        Ok(result)
    }

    // ---- evaluation ---------------------------------------------------------------

    /// `ContextIntProvider.getIntUnsafe`.
    pub(in crate::game) fn provider_int(
        &mut self,
        v: &Value,
        c: &ComputeContext,
        depth: u32,
    ) -> Result<i32, Arith> {
        if depth > 64 {
            return Ok(0);
        }
        let d = depth + 1;
        match v {
            Value::Number(n) => Ok(n
                .as_i64()
                .map_or_else(|| n.as_f64().unwrap_or(0.0) as i32, |i| i as i32)),
            Value::Bool(b) => Ok(i32::from(*b)),
            Value::String(id) => match self.provider_file("minecraft:context_int_provider", id) {
                Some(def) => self.provider_int(&def, c, d),
                None => Ok(0),
            },
            Value::Object(o) => {
                let ty = o
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("minecraft:")
                    .to_owned();
                let inputs = |g: &mut Self| -> Result<Vec<i32>, Arith> {
                    let list: Vec<Value> = match o.get("inputs") {
                        Some(Value::Array(a)) => a.clone(),
                        Some(other) => vec![other.clone()],
                        None => Vec::new(),
                    };
                    list.iter().map(|x| g.provider_int(x, c, d)).collect()
                };
                let field = |g: &mut Self, k: &str| -> Result<i32, Arith> {
                    match o.get(k) {
                        Some(x) => g.provider_int(x, c, d),
                        None => Ok(0),
                    }
                };
                let overflow = || "integer overflow".to_owned();
                let long_to_int = |v: i64| -> Result<i32, Arith> {
                    i32::try_from(v)
                        .map_err(|_| format!("Value {v} can't be safely converted to int"))
                };
                Ok(match ty.as_str() {
                    "constant" => field(self, "value")?,
                    "add" => long_to_int(inputs(self)?.iter().map(|&x| i64::from(x)).sum())?,
                    "mul" => long_to_int(
                        inputs(self)?
                            .iter()
                            .fold(1i64, |a, &x| a.wrapping_mul(i64::from(x))),
                    )?,
                    "avg" => {
                        let xs = inputs(self)?;
                        if xs.is_empty() {
                            return Err("/ by zero".into());
                        }
                        let sum: i64 = xs.iter().map(|&x| i64::from(x)).sum();
                        long_to_int(sum / xs.len() as i64)?
                    }
                    "max" => inputs(self)?.into_iter().fold(i32::MIN, i32::max),
                    "min" => inputs(self)?.into_iter().fold(i32::MAX, i32::min),
                    "sub" => {
                        let (a, b) = (field(self, "left")?, field(self, "right")?);
                        a.checked_sub(b).ok_or_else(overflow)?
                    }
                    "div" | "mod" | "floor_div" | "floor_mod" => {
                        let (a, b) = (field(self, "left")?, field(self, "right")?);
                        if b == 0 {
                            return Err("/ by zero".into());
                        }
                        match ty.as_str() {
                            "div" => a.wrapping_div(b),
                            "mod" => a.wrapping_rem(b),
                            "floor_mod" => {
                                a.rem_euclid(b) + if b < 0 && a.rem_euclid(b) != 0 { b } else { 0 }
                            }
                            _ => {
                                if a == i32::MIN && b == -1 {
                                    return Err(overflow());
                                }
                                a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 }
                            }
                        }
                    }
                    "pow" => {
                        let (base, exp) = (field(self, "base")?, field(self, "exponent")?);
                        if base == 0 && exp == 0 {
                            return Err("Result of 0 to the power of 0 is undefined".into());
                        }
                        if exp < 0 {
                            return Err("Negative exponent".into());
                        }
                        base.checked_pow(exp as u32).ok_or_else(overflow)?
                    }
                    // Math.absExact's message.
                    "abs" => field(self, "input")?.checked_abs().ok_or_else(|| {
                        "Overflow to represent absolute value of Integer.MIN_VALUE".to_owned()
                    })?,
                    "negate" => field(self, "input")?.checked_neg().ok_or_else(overflow)?,
                    "from_float" => {
                        let f = match o.get("input") {
                            Some(x) => self.provider_float(x, c, d)?,
                            None => 0.0,
                        };
                        if !f.is_finite() {
                            return Err(format!(
                                "Value {} can't be safely converted to int",
                                float_to_string(f)
                            ));
                        }
                        long_to_int(f as i64)?
                    }
                    "uniform" => {
                        let (lo, hi) = (field(self, "min")?, field(self, "max")?);
                        if lo >= hi {
                            lo
                        } else {
                            self.world.random.next_int_bound(hi - lo + 1).unwrap_or(0) + lo
                        }
                    }
                    "binomial" => {
                        let n = field(self, "n")?;
                        let p = match o.get("p") {
                            Some(x) => self.provider_float(x, c, d)?,
                            None => 0.0,
                        };
                        (0..n.max(0))
                            .filter(|_| self.world.random.next_float() < p)
                            .count() as i32
                    }
                    "conditional" | "number_dispatcher" | "weighted_list" => {
                        match self.provider_select(o, c)? {
                            Some(x) => self.provider_int(&x, c, d)?,
                            None => 0,
                        }
                    }
                    _ => 0,
                })
            }
            _ => Ok(0),
        }
    }
    /// `ContextFloatProvider.getFloatUnsafe`.
    pub(in crate::game) fn provider_float(
        &mut self,
        v: &Value,
        c: &ComputeContext,
        depth: u32,
    ) -> Result<f32, Arith> {
        if depth > 64 {
            return Ok(0.0);
        }
        let d = depth + 1;
        match v {
            Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0) as f32),
            Value::String(id) => match self.provider_file("minecraft:context_float_provider", id) {
                Some(def) => self.provider_float(&def, c, d),
                None => Ok(0.0),
            },
            Value::Object(o) => {
                let ty = o
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("minecraft:")
                    .to_owned();
                let inputs = |g: &mut Self| -> Result<Vec<f32>, Arith> {
                    let list: Vec<Value> = match o.get("inputs") {
                        Some(Value::Array(a)) => a.clone(),
                        Some(other) => vec![other.clone()],
                        None => Vec::new(),
                    };
                    list.iter().map(|x| g.provider_float(x, c, d)).collect()
                };
                let field = |g: &mut Self, k: &str| -> Result<f32, Arith> {
                    match o.get(k) {
                        Some(x) => g.provider_float(x, c, d),
                        None => Ok(0.0),
                    }
                };
                Ok(match ty.as_str() {
                    "constant" => field(self, "value")?,
                    "add" => inputs(self)?.iter().fold(0.0f32, |a, &x| a + x),
                    "mul" => inputs(self)?.iter().fold(1.0f32, |a, &x| a * x),
                    "avg" => {
                        let xs = inputs(self)?;
                        xs.iter().fold(0.0f32, |a, &x| a + x) / xs.len() as f32
                    }
                    "length" => inputs(self)?.iter().fold(0.0f32, |a, &x| a + x * x).sqrt(),
                    "max" => inputs(self)?.into_iter().fold(-f32::MAX, f32::max),
                    "min" => inputs(self)?.into_iter().fold(f32::MAX, f32::min),
                    "sub" => field(self, "left")? - field(self, "right")?,
                    "div" => field(self, "left")? / field(self, "right")?,
                    "mod" => {
                        let right = field(self, "right")?;
                        if right == 0.0 {
                            f32::NAN
                        } else {
                            field(self, "left")? % right
                        }
                    }
                    "pow" => {
                        let (base, exp) = (field(self, "base")?, field(self, "exponent")?);
                        if base == 0.0 && exp == 0.0 {
                            f32::NAN
                        } else {
                            f64::from(base).powf(f64::from(exp)) as f32
                        }
                    }
                    "abs" => field(self, "input")?.abs(),
                    "negate" => -field(self, "input")?,
                    "floor" => field(self, "input")?.floor(),
                    // Mth.ceil returns an int.
                    "ceil" => field(self, "input")?.ceil() as i32 as f32,
                    "round" => sunrise_core::jmath::round_f32(field(self, "input")?) as f32,
                    "truncate" => field(self, "input")?.trunc(),
                    "sqrt" => field(self, "input")?.sqrt(),
                    "sin" => sunrise_core::geometry::trig::sin(f64::from(field(self, "input")?)),
                    "cos" => sunrise_core::geometry::trig::cos(f64::from(field(self, "input")?)),
                    "from_int" => match o.get("input") {
                        Some(x) => self.provider_int(x, c, d)? as f32,
                        None => 0.0,
                    },
                    "uniform" => {
                        let (lo, hi) = (field(self, "min")?, field(self, "max")?);
                        if lo >= hi {
                            lo
                        } else {
                            self.world.random.next_float() * (hi - lo) + lo
                        }
                    }
                    "conditional" | "number_dispatcher" | "weighted_list" => {
                        match self.provider_select(o, c)? {
                            Some(x) => self.provider_float(&x, c, d)?,
                            None => 0.0,
                        }
                    }
                    _ => 0.0,
                })
            }
            _ => Ok(0.0),
        }
    }
    /// The branch a conditional, dispatcher or weighted list provider picks.
    fn provider_select(
        &mut self,
        o: &serde_json::Map<String, Value>,
        c: &ComputeContext,
    ) -> Result<Option<Value>, Arith> {
        match o
            .get("type")
            .and_then(Value::as_str)
            .map(|t| t.trim_start_matches("minecraft:"))
        {
            Some("conditional") => {
                let holds = o
                    .get("condition")
                    .is_some_and(|x| self.compute_condition(x, c));
                Ok(if holds {
                    o.get("on_true").cloned()
                } else {
                    Some(o.get("on_false").cloned().unwrap_or(Value::from(0)))
                })
            }
            Some("number_dispatcher") => {
                for case in o
                    .get("cases")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if case
                        .get("condition")
                        .is_some_and(|x| self.compute_condition(x, c))
                    {
                        return Ok(case.get("value").cloned());
                    }
                }
                Ok(Some(o.get("default").cloned().unwrap_or(Value::from(0))))
            }
            _ => {
                // WeightedList.getRandomOrThrow.
                let list: Vec<(Value, i32)> = o
                    .get("distribution")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        (
                            e.get("data").cloned().unwrap_or(Value::Null),
                            e.get("weight").and_then(Value::as_i64).unwrap_or(1) as i32,
                        )
                    })
                    .collect();
                let total: i32 = list.iter().map(|(_, w)| *w).sum();
                if total <= 0 {
                    return Ok(None);
                }
                let mut pick = self.world.random.next_int_bound(total).unwrap_or(0);
                for (v, w) in list {
                    pick -= w;
                    if pick < 0 {
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }
        }
    }
    /// A loot condition in the compute context: references, the logical
    /// combinators and `match_block` on the context's block state.
    fn compute_condition(&mut self, cond: &Value, c: &ComputeContext) -> bool {
        match cond {
            Value::String(id) => match self.provider_file("minecraft:predicate", id) {
                Some(def) => self.compute_condition(&def, c),
                None => false,
            },
            Value::Array(list) => list.iter().all(|x| self.compute_condition(x, c)),
            Value::Object(o) => {
                let ty = o
                    .get("type")
                    .or_else(|| o.get("condition"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("minecraft:");
                match ty {
                    "inverted" => !o.get("term").is_some_and(|x| self.compute_condition(x, c)),
                    "all_of" => o
                        .get("terms")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .all(|x| self.compute_condition(x, c)),
                    "any_of" => o
                        .get("terms")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|x| self.compute_condition(x, c)),
                    "match_block" | "block_state_property" => {
                        let Some(p) = c.block else {
                            return false;
                        };
                        let state = self.world.block(p[0], p[1], p[2]);
                        let Some(s) = self.pack.assets.registry.state(state) else {
                            return false;
                        };
                        let name = s.name.to_string();
                        let blocks_ok = match o.get("blocks").or_else(|| o.get("block")) {
                            Some(Value::String(b)) => match b.strip_prefix('#') {
                                Some(tag) => self
                                    .res
                                    .items()
                                    .block_tags
                                    .get(tag.trim_start_matches("minecraft:"))
                                    .is_some_and(|set| {
                                        set.contains(&*name)
                                            || set.contains(name.trim_start_matches("minecraft:"))
                                    }),
                                None => *b == name,
                            },
                            Some(Value::Array(list)) => {
                                list.iter().any(|b| b.as_str() == Some(&name))
                            }
                            _ => true,
                        };
                        let props_ok = o
                            .get("state")
                            .or_else(|| o.get("properties"))
                            .and_then(Value::as_object)
                            .into_iter()
                            .flatten()
                            .all(|(k, v)| s.properties.get(k).map(String::as_str) == v.as_str());
                        blocks_ok && props_ok
                    }
                    "random_chance" => {
                        let chance = o.get("chance").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        self.world.random.next_float() < chance
                    }
                    _ => {
                        let source = self.command_source.unwrap_or_else(|| self.player_source());
                        // Scores are read from the holder's board only where
                        // an `entity_scores` leaf may ask.
                        let scored = adv::asks_scores(cond);
                        let view = |game: &mut Self, t| {
                            if scored {
                                game.entity_view_scored(t, cond)
                            } else {
                                game.entity_view_asked(t, cond)
                            }
                        };
                        let this = source
                            .entity
                            .filter(|_| c.without_this.is_none())
                            .map(|t| view(self, t));
                        let target = c.target.map(|t| view(self, t));
                        let interacting = c.interacting.map(|t| view(self, t));
                        let entities = context_entities(
                            c,
                            this.as_ref(),
                            source.pos,
                            target.as_ref(),
                            interacting.as_ref(),
                        );
                        self.conditions_hold_in(cond, &entities)
                    }
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A world with nothing in it: the entity conditions under test read none of it.
    struct Void;
    impl adv::World for Void {
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn weather(&self) -> (bool, bool) {
            (false, false)
        }
        fn biome(&self, _: [i32; 3]) -> String {
            String::new()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:air".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            false
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            0
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, _: &str, _: &str) -> bool {
            false
        }
    }

    fn view(kind: &str, position: [f64; 3]) -> EntityView {
        EntityView {
            kind: kind.into(),
            position,
            ..Default::default()
        }
    }

    fn holds(cond: &Value, entities: &adv::Entities) -> bool {
        adv::conditions(cond, entities, &Void)
    }

    fn of_type(who: &str, kind: &str) -> Value {
        json!({"condition": "minecraft:entity_properties", "entity": who,
            "predicate": {"minecraft:entity_type": kind}})
    }

    /// A command context's `this` and origin are the source's; `target_entity`
    /// and `interacting_entity` are the ones the context carries.
    #[test]
    fn a_compute_context_selects_the_entities_it_carries() {
        let (source, target, actor) = (
            view("minecraft:player", [0.0; 3]),
            view("minecraft:cow", [0.0; 3]),
            view("minecraft:zombie", [0.0; 3]),
        );
        let c = ComputeContext::default();
        let e = context_entities(
            &c,
            Some(&source),
            [1.0, 2.0, 3.0],
            Some(&target),
            Some(&actor),
        );
        assert_eq!(e.origin, [1.0, 2.0, 3.0]);
        assert!(holds(&of_type("this", "minecraft:player"), &e));
        assert!(holds(&of_type("target_entity", "minecraft:cow"), &e));
        assert!(!holds(&of_type("target_entity", "minecraft:zombie"), &e));
        assert!(holds(
            &of_type("interacting_entity", "minecraft:zombie"),
            &e
        ));
        let bare = context_entities(&c, Some(&source), [0.0; 3], None, None);
        assert!(!holds(&of_type("target_entity", "minecraft:cow"), &bare));
        assert!(!holds(
            &of_type("interacting_entity", "minecraft:zombie"),
            &bare
        ));
    }

    /// `BLOCK_INTERACT` (the composter) has no `THIS_ENTITY` and measures from
    /// the block's centre, whoever ran the command or whatever the source is.
    #[test]
    fn a_context_without_this_measures_from_its_own_origin() {
        let source = view("minecraft:player", [0.5, 64.0, 0.5]);
        let actor = view("minecraft:player", [10.5, 64.0, 0.5]);
        let c = ComputeContext {
            without_this: Some([10.5, 64.0, 0.5]),
            ..ComputeContext::default()
        };
        let e = context_entities(&c, Some(&source), source.position, None, Some(&actor));
        assert!(!holds(&of_type("this", "minecraft:player"), &e));
        assert!(holds(
            &of_type("interacting_entity", "minecraft:player"),
            &e
        ));
        // The interacting player stands on the origin, ten blocks from the source.
        let near = json!({"condition": "minecraft:entity_properties", "entity": "interacting_entity",
            "predicate": {"minecraft:distance": {"absolute": {"max": 1.0}}}});
        assert!(holds(&near, &e));
        let from_source = context_entities(
            &ComputeContext::default(),
            Some(&source),
            source.position,
            None,
            Some(&actor),
        );
        assert!(!holds(&near, &from_source));
    }

    #[test]
    #[ignore = "daily"]
    fn published_predicates_and_nested_provider_shorthand_evaluate() {
        use crate::game::*;
        use serde_json::json;
        use std::{path::PathBuf, sync::Arc, time::Instant};
        use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};

        let root = PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let mut loaded = load_resources(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let fixture = std::env::current_dir()
            .unwrap()
            .join("compute-owner-providers");
        for (registry, value) in [
            ("context_int_provider", json!(17)),
            ("context_float_provider", json!(2.5)),
        ] {
            let dir = fixture.join("data/minecraft").join(registry).join("nested");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("value.json"), value.to_string()).unwrap();
        }
        Arc::get_mut(&mut loaded.resources)
            .unwrap()
            .rest
            .get_mut()
            .unwrap()
            .providers = sunrise_client::providers::Providers::load(&fixture);
        std::fs::remove_dir_all(&fixture).unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&ResourceSource::directory(root)),
            None,
        )
        .unwrap();
        game.fns.store.predicates.insert(
            "minecraft:nested/true".into(),
            json!({"condition": "minecraft:all_of", "terms": []}),
        );
        let context = ComputeContext::default();
        for id in ["nested/value", "minecraft:nested/value"] {
            assert_eq!(
                game.provider_int(
                    &json!({"type": "minecraft:add", "inputs": [id, 3]}),
                    &context,
                    0
                ),
                Ok(20),
            );
            assert_eq!(
                game.provider_float(
                    &json!({"type": "minecraft:add", "inputs": [id, 0.5]}),
                    &context,
                    0
                ),
                Ok(3.0),
            );
        }
        for predicate in ["nested/true", "minecraft:nested/true"] {
            assert_eq!(
                game.provider_int(
                    &json!({
                        "type": "minecraft:conditional", "condition": predicate,
                        "on_true": "nested/value", "on_false": -1
                    }),
                    &context,
                    0,
                ),
                Ok(17),
            );
            assert_eq!(
                game.provider_float(
                    &json!({
                        "type": "minecraft:number_dispatcher",
                        "cases": [{"condition": predicate, "value": "nested/value"}],
                        "default": -1
                    }),
                    &context,
                    0,
                ),
                Ok(2.5),
            );
        }
        // `/compute entity`: `target_entity` is the selected entity, and the
        // source (here with no entity) is only `this`.
        game.set_allow_commands(true);
        assert!(game.run_command("scoreboard objectives add points dummy"));
        game.player = Some(
            sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            )
            .unwrap(),
        );
        assert!(game.run_command("scoreboard players set @s points 5"));
        let mut source = game.player_source();
        source.entity = None;
        game.command_source = Some(source);
        let on_target = |kind: &str| {
            json!({
                "type": "minecraft:conditional",
                "condition": {"condition": "minecraft:entity_properties", "entity": "target_entity",
                    "predicate": {"minecraft:entity_type": kind}},
                "on_true": 1, "on_false": 0
            })
        };
        let entity = ComputeContext {
            target: Some(Target::Player),
            ..ComputeContext::default()
        };
        assert_eq!(
            game.provider_int(&on_target("minecraft:player"), &entity, 0),
            Ok(1)
        );
        assert_eq!(
            game.provider_int(&on_target("minecraft:cow"), &entity, 0),
            Ok(0)
        );
        assert_eq!(
            game.provider_int(
                &on_target("minecraft:player"),
                &ComputeContext::default(),
                0
            ),
            Ok(0)
        );
        // `entity_scores` reads the selected holder's live scoreboard value.
        let scored = |n: i32| {
            json!({
                "type": "minecraft:conditional",
                "condition": {"condition": "minecraft:entity_scores", "entity": "target_entity",
                    "scores": {"points": n}},
                "on_true": 1, "on_false": 0
            })
        };
        assert_eq!(game.provider_int(&scored(5), &entity, 0), Ok(1));
        assert_eq!(game.provider_int(&scored(4), &entity, 0), Ok(0));
    }
}
