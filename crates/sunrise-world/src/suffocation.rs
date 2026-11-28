//! Native suffocation predicates bound to a registry and a resolved block-tag set.
use crate::{
    block_collision::{BlockCollisionRegistry, CollisionQueryError, DynamicCollisionWorld},
    compression::invalid,
    registry::BlockStateId,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use sunrise_core::geometry::{ShapeOp, VoxelShape};

#[derive(Clone, Copy, Debug)]
pub enum SuffocationDefinition {
    TaggedFullCollision,
    Always,
    Never,
    NotExtendedPiston,
    ClosedShulker,
}
#[derive(Clone, Copy, Debug)]
enum Rule {
    Constant(bool),
    DynamicFull,
    Shulker,
}

pub struct SuffocationRegistry<'r> {
    collisions: Arc<BlockCollisionRegistry<'r>>,
    rules: Vec<Rule>,
}
impl<'r> SuffocationRegistry<'r> {
    pub fn collisions(&self) -> &BlockCollisionRegistry<'r> {
        &self.collisions
    }

    /// Rebind after the causes_suffocation tag changes. Definition keys and tag
    /// members are canonical block identifiers from the same pinned registry.
    pub fn new(
        collisions: Arc<BlockCollisionRegistry<'r>>,
        definitions: &BTreeMap<String, SuffocationDefinition>,
        causes_suffocation: &BTreeSet<String>,
    ) -> io::Result<Self> {
        let registry = collisions.registry();
        if definitions
            .keys()
            .chain(causes_suffocation)
            .any(|name| registry.default_state(name).is_none())
        {
            return Err(invalid("unknown suffocation definition or tag member"));
        }
        let mut full_shapes = HashMap::new();
        let mut rules = Vec::with_capacity(registry.len());
        for index in 0..registry.len() {
            let id = BlockStateId(index as u32);
            let state = registry.state(id).unwrap();
            let definition = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing suffocation definition"))?;
            let rule = match definition {
                SuffocationDefinition::Always => Rule::Constant(true),
                SuffocationDefinition::Never => Rule::Constant(false),
                SuffocationDefinition::NotExtendedPiston => {
                    Rule::Constant(match state.properties.get("extended").map(String::as_str) {
                        Some("false") => true,
                        Some("true") => false,
                        _ => return Err(invalid("invalid piston suffocation property")),
                    })
                }
                SuffocationDefinition::ClosedShulker => Rule::Shulker,
                SuffocationDefinition::TaggedFullCollision => {
                    if !causes_suffocation.contains(&state.name) {
                        Rule::Constant(false)
                    } else if collisions.publishes_shapes(id)
                        || collisions.cached_shapes().offset_collision(id)
                    {
                        // Publication and declared offsets make the full-shape answer positional.
                        Rule::DynamicFull
                    } else if let Some(shape) = collisions.cached_shapes().cached_shape(id)? {
                        let identity = std::ptr::from_ref(shape);
                        let full = if let Some(full) = full_shapes.get(&identity) {
                            *full
                        } else {
                            let full = !shape
                                .join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)
                                .map_err(|e| invalid(e.to_string()))?;
                            full_shapes.insert(identity, full);
                            full
                        };
                        Rule::Constant(full)
                    } else {
                        Rule::DynamicFull
                    }
                }
            };
            rules.push(rule);
        }
        Ok(Self { collisions, rules })
    }

    /// The answer for `state` when it does not depend on the position or a block entity: the
    /// rule is constant. None for a closed-shulker rule, a full-shape test that reads a published
    /// or offset shape, and a state outside the registry.
    pub fn constant(&self, state: BlockStateId) -> Option<bool> {
        match self.rules.get(state.0 as usize)? {
            Rule::Constant(value) => Some(*value),
            Rule::DynamicFull | Rule::Shulker => None,
        }
    }

    pub fn is_suffocating<W: DynamicCollisionWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, CollisionQueryError<W::Error>> {
        let rule = self.rules.get(state.0 as usize).ok_or_else(|| {
            CollisionQueryError::Data(invalid("suffocation state outside registry"))
        })?;
        match rule {
            Rule::Constant(value) => Ok(*value),
            Rule::Shulker => world
                .shulker_is_closed(position)
                .map(|closed| closed != Some(false))
                .map_err(CollisionQueryError::Context),
            Rule::DynamicFull => self
                .collisions
                .collision_shape(world, state, position)?
                .join_is_not_empty(&VoxelShape::block(), ShapeOp::XOR)
                .map(|different| !different)
                .map_err(|error| CollisionQueryError::Data(invalid(error.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DATA_VERSION, collision::CachedCollisionRegistry, registry::BlockRegistry};
    use sunrise_core::nbt::Tag;

    /// `constant` answers a rule that reads neither the position nor a block entity, and nothing
    /// for a closed-shulker rule, a full-shape test of a state with no cached shape, and a state
    /// outside the registry.
    #[test]
    fn constant_answers_only_rules_that_read_no_position() {
        let names = [
            "minecraft:air",
            "minecraft:stone",
            "test:context",
            "test:box",
        ];
        let states = Tag::List(
            names
                .iter()
                .map(|name| Tag::String((*name).into()))
                .collect(),
        );
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", states.clone()),
            (
                "defaults",
                Tag::compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(index, name)| (*name, Tag::Int(index as i32))),
                ),
            ),
        ]))
        .unwrap();
        let empty = Tag::compound([
            ("x", Tag::LongArray(vec![0])),
            ("y", Tag::LongArray(vec![0])),
            ("z", Tag::LongArray(vec![0])),
            ("occupied", Tag::ByteArray(vec![])),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![0; 3])),
        ]);
        let coords = Tag::LongArray(vec![0, 1.0f64.to_bits() as i64]);
        let block = Tag::compound([
            ("x", coords.clone()),
            ("y", coords.clone()),
            ("z", coords),
            ("occupied", Tag::ByteArray(vec![1])),
            ("cube", Tag::Byte(1)),
            ("cube_axes", Tag::Byte(7)),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![1; 3])),
        ]);
        // Air is empty, stone and the box a full cube; the context state has no cached shape.
        let collision = Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", states),
            ("shapes", Tag::List(vec![empty, block])),
            ("cached_shapes", Tag::IntArray(vec![0, 1, -1, 1])),
        ]);
        let cached = Arc::new(CachedCollisionRegistry::from_nbt(&collision, &registry).unwrap());
        let collisions = Arc::new(BlockCollisionRegistry::new(cached).unwrap());
        let definitions = [
            ("minecraft:air", SuffocationDefinition::Never),
            (
                "minecraft:stone",
                SuffocationDefinition::TaggedFullCollision,
            ),
            ("test:context", SuffocationDefinition::TaggedFullCollision),
            ("test:box", SuffocationDefinition::ClosedShulker),
        ]
        .map(|(name, definition)| (name.to_owned(), definition))
        .into_iter()
        .collect();
        let tagged = ["minecraft:stone", "test:context"]
            .map(str::to_owned)
            .into_iter()
            .collect();
        let rules = SuffocationRegistry::new(collisions, &definitions, &tagged).unwrap();
        assert_eq!(rules.constant(BlockStateId(0)), Some(false));
        assert_eq!(rules.constant(BlockStateId(1)), Some(true));
        // A full-shape test with no cached shape reads the shape at a position.
        assert_eq!(rules.constant(BlockStateId(2)), None);
        // A closed-shulker rule reads the block entity at a position.
        assert_eq!(rules.constant(BlockStateId(3)), None);
        assert_eq!(rules.constant(BlockStateId(names.len() as u32)), None);
    }
}
