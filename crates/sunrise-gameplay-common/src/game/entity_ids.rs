//! Entity ids are `u32` storage bits; the wire, NBT and the mod API carry them
//! as `i32`, so an id with the high bit set is stored negative and must be
//! converted back through its bits, never by a checked narrowing (#6335, #6336).
//!
//! Signed fields such as `HurtSource::causing` and a guest's `entity_id` carry
//! those storage bits; `EntityRef` and `LivingRef` expose the `u32` id.

/// The `i32` an entity id is stored as on the wire and in NBT.
pub const fn wire_entity_id(id: u32) -> i32 {
    id as i32
}

/// The `u32` storage id a wire or API `i32` carries, high bit included.
pub const fn entity_id_bits(id: i32) -> u32 {
    id as u32
}

#[cfg(test)]
mod tests {
    use super::{entity_id_bits, wire_entity_id};

    #[test]
    fn the_storage_encoding_round_trips_a_high_bit_id() {
        let id = 0x8000_0001_u32;
        assert_eq!(wire_entity_id(id), -2_147_483_647);
        assert_eq!(entity_id_bits(wire_entity_id(id)), id);
        assert_eq!(entity_id_bits(-1), u32::MAX);
    }
}
