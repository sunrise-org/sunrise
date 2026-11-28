pub trait Notes {
    #[notes = r#"## Equipment slot

`ItemResource::equippable_slot(host)` answers which slot a stack goes in when worn
(`Equippable.slot`), from the item's `equippable` component after the stack's patch, so a
command's or a data pack's override counts. It does not say whether a given entity may wear it;
that depends on the wearer and is not answered here."#]
    fn item_components_note0();
    #[notes = r#"## Enchantment conflicts

`Host::enchantments_compatible(a, b)` answers `Enchantment.areCompatible` from the world's
enchantment definitions, so a data pack's `exclusive_set` counts. A mod that adds an enchantment
to a stack asks it for each enchantment already on it, as the enchanting table and the anvil do."#]
    fn item_components_note1();
    #[notes = r#"## Effective stack properties

`ItemResource` reads the stack's component overrides and removals before item defaults:
`damageable(host)`, `foil(host)`, `rarity(host)` and `enchantable(host)`.
`repairable(host, material)` takes another resource; only its item identity and tags matter,
not its count or component patch. `bar(host)` returns an optional `ItemBar`, with width out of
13 pixels and RGB colour, before client drawing hooks; bundles use their fullness bar.
`use_duration(host)` returns ticks, including the gameplay use-duration Part. Vanilla's
current item classes do not require a player argument; Parts see the host's current acting
player. These reads use a world-phase host; tag-based repair checks need its authoritative world tags.
Duration and bar reads need an engine backend and return `None` for an unknown item."#]
    fn item_components_note2();
    #[notes = r#"## Tooltip lines and random enchanting

`ItemResource::tooltip(client, advanced, creative)` returns styled text and line colours
for overlays that need the ordinary inventory tooltip. It uses the client's current viewer
and the requested flags. Queries include mod edits, except during item tooltip line construction,
when they return base lines. A query never gathers images or lays out or draws a tooltip.

`ItemResource::enchant_randomly(server, level, power, pool)` rolls from the specified
enchantment tag with the active server level's random source and the stack's effective
enchantability. It returns a detached result. Use the normal item-place or capability
transaction to replace a stored stack. Discarding the result does not rewind the random source."#]
    fn item_components_note3();
    #[notes = r#"## Stack combat queries

`ItemResource::entity_attack_range(host, attacker)` returns effective minimum, maximum and
hitbox margin; `contains(distance, buffer)` uses vanilla's inclusive float bounds.
`damage_source(host, attacker)` returns a typed owned source: component override, item-specific
source (mace smash), then the actor's ordinary melee source. Actors and targets are typed handles;
unavailable or stale handles return `None`. These reads never equip the queried stack.
`can_be_hurt_by(host, source)` asks whether the item itself can take that source's damage,
including damage-resistant component overrides, removals and damage type tags. It does not
calculate the wearer's protection.
`attack_damage_bonus(host, attacker, target, base_damage)` previews enchantment and item bonuses
with a cloned RNG snapshot. A read never consumes world RNG; a later attack may roll differently.
`modify_knockback(host, victim, damage_type, direct, causing, base)` is
`EnchantmentHelper.modifyKnockback`: `base` after the stack's `minecraft:knockback` effects, with
the damage source built as `EntityRef::hurt` builds it from a type id and its two entities. As in
vanilla, requirements are checked on the level's random and values applied on the victim's own;
the read runs on a copy of each, so neither stream moves, and it never equips the stack. The
victim's, attacker's and direct entity's fall distance and flying state are read as an attack
reads them. It is server only: a client, a stale victim, an unknown item, an unregistered damage
type, or a victim, `direct` or `causing` in another level than the one in place returns `None`.
`hurt_and_break` remains the durability write."#]
    fn item_components_note4();
}
