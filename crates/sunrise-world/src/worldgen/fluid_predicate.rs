//! Resolved fluid holder sets bound to immutable vanilla block-state identities.
use super::{DecodeError, density::identifier};
use crate::{
    fluid::{FluidKind, FluidStateTable},
    registry::{BlockRegistry, BlockStateId},
};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone)]
pub struct FluidPredicateBindings {
    table: Arc<FluidStateTable>,
    tags: BTreeMap<String, u8>,
}
impl FluidPredicateBindings {
    pub fn new(registry: &BlockRegistry) -> Result<Self, DecodeError> {
        Ok(Self {
            table: Arc::new(
                FluidStateTable::new(registry).map_err(|e| DecodeError::Registry(e.to_string()))?,
            ),
            tags: BTreeMap::new(),
        })
    }
    pub fn with_resolved_tags(
        mut self,
        tags: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self, DecodeError> {
        let mut resolved = BTreeMap::new();
        for (tag, names) in tags {
            let mut mask = 0;
            for name in names {
                mask |= fluid(name)?.mask();
            }
            if resolved
                .insert(super::tags::identifier(tag)?, mask)
                .is_some()
            {
                return Err(DecodeError::Field("duplicate fluid tag"));
            }
        }
        self.tags = resolved;
        Ok(self)
    }
    pub(crate) fn table(&self) -> &Arc<FluidStateTable> {
        &self.table
    }
    pub fn state(&self, block: BlockStateId) -> Option<crate::fluid::FluidState> {
        self.table.get(block)
    }
    /// Missing tags have no members, as for a native tag-key membership query.
    pub fn state_matches_tag(&self, block: BlockStateId, tag: &str) -> bool {
        self.table.get(block).is_some_and(|state| {
            self.tags
                .get(tag)
                .is_some_and(|mask| state.kind().mask() & mask != 0)
        })
    }
    /// A fluid holder set in any form `crate::holder_set` reads, as a mask.
    pub fn resolve(&self, json: &str) -> Result<BoundFluidSet, DecodeError> {
        let set: serde_json::Value = serde_json::from_str(json)?;
        let mask = crate::holder_set::resolve(&set, self)
            .map_err(DecodeError::Registry)?
            .into_iter()
            .fold(0, |mask, bit| mask | bit);
        Ok(BoundFluidSet {
            table: self.table.clone(),
            mask,
        })
    }
}
/// Members are single mask bits, so `not` complements the mask.
impl crate::holder_set::HolderLookup for FluidPredicateBindings {
    type Id = u8;
    fn what(&self) -> &str {
        "fluid"
    }
    fn entry(&self, id: &str) -> Result<Vec<u8>, String> {
        fluid(id)
            .map(|kind| bits(kind.mask()))
            .map_err(|e| e.to_string())
    }
    fn tag(&self, tag: &str) -> Result<Vec<u8>, String> {
        let tag = super::tags::identifier(tag).map_err(|e| e.to_string())?;
        self.tags
            .get(&tag)
            .map(|mask| bits(*mask))
            .ok_or_else(|| format!("unknown fluid tag {tag}"))
    }
    fn all(&self) -> Result<Vec<u8>, String> {
        Ok(bits(u8::MAX))
    }
}
fn bits(mask: u8) -> Vec<u8> {
    (0..8)
        .map(|bit| 1 << bit)
        .filter(|bit| mask & bit != 0)
        .collect()
}
fn fluid(name: &str) -> Result<FluidKind, DecodeError> {
    FluidKind::from_name(&identifier(name)?)
        .ok_or_else(|| DecodeError::Registry(format!("unknown fluid {name}")))
}

/// A fluid-family mask over one shared immutable state table. Membership needs
/// one indexed lookup and avoids allocating large sets of equivalent states.
#[derive(Clone, Debug)]
pub struct BoundFluidSet {
    table: Arc<FluidStateTable>,
    mask: u8,
}
impl BoundFluidSet {
    pub fn contains(&self, state: BlockStateId) -> bool {
        self.table
            .get(state)
            .is_some_and(|value| value.kind().mask() & self.mask != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{
        block_feature::FeatureBlockBindings, block_predicate::BlockPredicate,
        generator::data::load_fluid_tags, overrides::WorldgenOverrides,
    };
    use std::collections::BTreeSet;

    /// Air, the two air variants the bindings need, and one source state per fluid
    /// family, so a tag can name a member and a non-member.
    fn registry() -> BlockRegistry {
        let mut rows = Vec::new();
        let mut defaults = std::collections::BTreeMap::new();
        for name in ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"] {
            defaults.insert(name.to_owned(), rows.len() as i64);
            rows.push(serde_json::json!([name, true, false, "",
                {"id": name, "properties": {}}]));
        }
        for name in ["minecraft:water", "minecraft:lava"] {
            defaults.insert(name.to_owned(), rows.len() as i64);
            rows.push(serde_json::json!([name, false, true, "",
                {"id": name, "properties": {"level": "0"}}]));
        }
        BlockRegistry::from_facts_json(
            serde_json::json!({"states": rows, "defaults": defaults})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn fluid_tags_above_8192_load_and_match_through_the_real_loader() {
        let registry = registry();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();

        // A prepared `fluid_tags.json` of more definitions than 8,192, read by the
        // production loader. The last tag names lava, every other one water.
        let mut prepared = BTreeMap::new();
        for i in 0..8200 {
            let values = if i == 8199 {
                r#"["minecraft:lava"]"#
            } else {
                r#"["minecraft:water"]"#
            };
            prepared.insert(format!("test:tag_{i}"), format!(r#"{{"values":{values}}}"#));
        }
        let scratch = tempfile::tempdir().unwrap();
        std::fs::write(
            scratch.path().join("fluid_tags.json"),
            serde_json::to_string(&prepared).unwrap(),
        )
        .unwrap();
        // A pack appends lava to the first tag, so override merging is on the path.
        let mut overrides = WorldgenOverrides::default();
        overrides.add_tag(
            "fluid_tags",
            "test:tag_0",
            "pack",
            r#"{"values":["minecraft:lava"]}"#.to_owned(),
        );
        let mut warnings = Vec::new();
        let tags = load_fluid_tags(scratch.path(), &overrides, &mut warnings)
            .expect("the real loader accepts past 8,192 fluid tag definitions");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(tags.len() >= 8200);

        let blocks = FeatureBlockBindings::new(&registry, BTreeSet::new())
            .and_then(|bindings| bindings.with_fluid_tags(&tags))
            .unwrap();

        // `matching_fluids` decodes the high-numbered tag into a bound set that
        // then answers membership exactly, through the predicate decoder.
        let decode = |tag: &str| {
            let json = format!(r##"{{"type":"minecraft:matching_fluids","fluids":"#{tag}"}}"##);
            BlockPredicate::from_json_with_blocks(&json, &blocks).unwrap()
        };
        for (tag, member, non_member) in [
            ("test:tag_8198", water, Some(lava)),
            ("test:tag_8199", lava, Some(water)),
            ("test:tag_0", water, None),
            ("test:tag_0", lava, None),
        ] {
            let BlockPredicate::MatchingFluids { fluids, .. } = decode(tag) else {
                panic!("{tag} did not decode to matching_fluids");
            };
            assert!(fluids.contains(member), "{tag} matches its member");
            if let Some(non_member) = non_member {
                assert!(!fluids.contains(non_member), "{tag} rejects a non-member");
            }
        }
        assert!(blocks.fluids.resolve("\"#test:tag_8200\"").is_err());

        // An explicit holder list past 8,192 entries still resolves to the same
        // exact membership as a tag would.
        let explicit = format!(
            "[{}]",
            (0..8200)
                .map(|_| "\"minecraft:water\"")
                .collect::<Vec<_>>()
                .join(",")
        );
        let set = blocks.fluids.resolve(&explicit).unwrap();
        assert!(set.contains(water));
        assert!(!set.contains(lava));
    }
}
