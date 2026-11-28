//! One shared startup `(kind, id) -> (owner_mod, TypeId, type_name)` identity guard
//! across capabilities, custom static registries, mod events, and data maps.
use std::{any::TypeId, collections::HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeIdentityKind {
    BlockCapability,
    ItemCapability,
    EntityCapability,
    CustomRegistry,
    ModEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypeIdentity {
    type_id: TypeId,
    type_name: &'static str,
}

impl TypeIdentity {
    pub(crate) fn typed(type_id: TypeId, type_name: &'static str) -> Self {
        Self { type_id, type_name }
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    owner_mod: &'static str,
    identity: TypeIdentity,
}

/// Startup table recording the first Rust type identity bound to each `(kind, id)`.
///
/// Any later registration under the same `(kind, id)` with a different type identity is
/// refused with a diagnostic naming the kind, ID, both mods, and both Rust type names.
#[derive(Clone, Default)]
pub(crate) struct TypeIdentityTable {
    entries: HashMap<(TypeIdentityKind, String), Entry>,
}

impl TypeIdentityTable {
    /// Seed a built-in binding (owned by `"the game"`) without refusing.
    pub(crate) fn seed(&mut self, kind: TypeIdentityKind, id: &str, identity: TypeIdentity) {
        self.entries.insert(
            (kind, id.to_owned()),
            Entry {
                owner_mod: "the game",
                identity,
            },
        );
    }

    /// Record `(kind, id)` for `mod_id` with `identity`, or return a refusal naming
    /// both mods and both types if an incompatible identity was already bound.
    pub(crate) fn claim(
        &mut self,
        kind: TypeIdentityKind,
        id: &str,
        mod_id: &'static str,
        identity: TypeIdentity,
    ) -> Result<(), String> {
        let key = (kind, id.to_owned());
        if let Some(existing) = self.entries.get(&key) {
            if existing.identity.type_id == identity.type_id {
                return Ok(());
            }
            return match kind {
                TypeIdentityKind::BlockCapability
                | TypeIdentityKind::ItemCapability
                | TypeIdentityKind::EntityCapability => {
                    let conflict = if existing.identity.type_name == identity.type_name {
                        "incompatible handler or context types"
                    } else {
                        "two handler types"
                    };
                    Err(format!(
                        "capability {id} is answered with {conflict}, {} by {} and {} by {mod_id}",
                        existing.identity.type_name, existing.owner_mod, identity.type_name
                    ))
                }
                TypeIdentityKind::CustomRegistry => Err(format!(
                    "custom registry {id} is used with two types, {} by {} and {} by {mod_id}",
                    existing.identity.type_name, existing.owner_mod, identity.type_name
                )),
                TypeIdentityKind::ModEvent => Err(format!(
                    "event {id} is listened to with two types, {} by {} and {} by {mod_id}",
                    existing.identity.type_name, existing.owner_mod, identity.type_name
                )),
            };
        }
        self.entries.insert(
            key,
            Entry {
                owner_mod: mod_id,
                identity,
            },
        );
        Ok(())
    }
}
