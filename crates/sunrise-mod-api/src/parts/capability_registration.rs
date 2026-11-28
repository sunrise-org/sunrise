//! One live registration table for provider type coverage and forwarding metadata.
use super::*;
use std::{any::TypeId, collections::hash_map::Entry};
use sunrise_mod_runtime::registry_keys::Names;

impl PartRegistrar {
    pub(crate) fn capability_parts() -> (Parts, TypeIdentityTable) {
        let mut parts = Parts::default();
        let mut type_identities = TypeIdentityTable::default();
        for (by, name, binding) in mod_vocab::__private::parts_installation::builtin_capabilities()
        {
            let (kind, metadata) = match by {
                ProvidedBy::Block | ProvidedBy::BlockEntity => {
                    parts.block_capability_order.push(name);
                    (
                        TypeIdentityKind::BlockCapability,
                        &mut parts.block_capability_metadata,
                    )
                }
                ProvidedBy::Item => (
                    TypeIdentityKind::ItemCapability,
                    &mut parts.item_capability_metadata,
                ),
                ProvidedBy::Entity => (
                    TypeIdentityKind::EntityCapability,
                    &mut parts.entity_capability_metadata,
                ),
            };
            type_identities.seed(kind, name, TypeIdentity::typed(binding.0, binding.1));
            metadata.insert(
                name,
                CapabilityMetadata {
                    binding,
                    owner: "the game",
                    proxyable: None,
                },
            );
        }
        (parts, type_identities)
    }

    /// Registry requests already carry validated names, including forward references whose
    /// existence the completed registry will check at load.
    pub(crate) fn capability_key_target<'a>(
        &mut self,
        name: &str,
        kind: &str,
        registry: &'a Names,
        slot: u32,
    ) -> Option<&'a str> {
        let id = registry.id(slot);
        if id.is_none() {
            self.refused.push(format!(
                "mod {} used capability {name:?} for an invalid {kind} key slot {slot}",
                self.mod_id
            ));
        }
        id
    }

    /// Like NeoForge's `CapabilityRegistry.create`, the name's first binding fixes the handler
    /// and context identities in that domain, even if nobody has provided a handler yet.
    pub(crate) fn note_capability(
        &mut self,
        by: ProvidedBy,
        name: &'static str,
        binding: (TypeId, &'static str),
    ) -> Option<&mut CapabilityMetadata> {
        if crate::Identifier::parse_namespaced(name).is_none() {
            self.refused.push(format!(
                "mod {} used capability {name:?}, which is not a namespaced name",
                self.mod_id
            ));
            return None;
        }
        let (kind, metadata, block_order) = match by {
            ProvidedBy::Block | ProvidedBy::BlockEntity => (
                TypeIdentityKind::BlockCapability,
                &mut self.parts.block_capability_metadata,
                Some(&mut self.parts.block_capability_order),
            ),
            ProvidedBy::Item => (
                TypeIdentityKind::ItemCapability,
                &mut self.parts.item_capability_metadata,
                None,
            ),
            ProvidedBy::Entity => (
                TypeIdentityKind::EntityCapability,
                &mut self.parts.entity_capability_metadata,
                None,
            ),
        };
        if let Err(error) = self.type_identities.claim(
            kind,
            name,
            self.mod_id,
            TypeIdentity::typed(binding.0, binding.1),
        ) {
            self.refused.push(error);
            return None;
        }
        match metadata.entry(name) {
            Entry::Vacant(entry) => {
                if let Some(order) = block_order {
                    order.push(name);
                }
                Some(entry.insert(CapabilityMetadata {
                    binding,
                    owner: self.mod_id,
                    proxyable: None,
                }))
            }
            Entry::Occupied(entry) => Some(entry.into_mut()),
        }
    }

    fn block_capability_covers(&self, name: &str, id: &str) -> bool {
        let declared_default = if name == crate::ITEMS.name {
            self.parts.default_block_containers.contains_key(id)
        } else if name == crate::FLUIDS.name {
            self.parts.default_block_fluids.contains_key(id)
        } else if name == crate::ENERGY.name {
            self.parts.default_block_energy.contains_key(id)
        } else {
            false
        };
        declared_default
            || self
                .parts
                .block_capabilities
                .get(name)
                .and_then(|types| types.get(id))
                .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Block,
                name,
                id,
            )
            || self.parts.capabilities.get(name).is_some_and(|types| {
                types.iter().any(|(kind, providers)| {
                    !providers.is_empty()
                        && (self.block_entity_types.iter().any(|(_, declaration)| {
                            declaration.id == **kind
                                && declaration.blocks.iter().any(|block| block == id)
                        }) || mod_vocab::__private::parts_installation::block_entity_covers(
                            kind, id,
                        ))
                })
            })
            || mod_vocab::__private::parts_installation::block_capability_dependency(name)
                .is_some_and(|source| self.block_capability_covers(source, id))
    }
}

impl CapabilityRegistrationRegistrar for PartRegistrar {
    fn entity_type_key(&mut self, id: &str) -> crate::EntityTypeKey {
        match self
            .registry_keys
            .borrow_mut()
            .entity_types
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::entity_type_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::entity_type_key(u32::MAX)
            }
        }
    }

    fn block_capability_registered(
        &mut self,
        name: &'static str,
        binding: (TypeId, &'static str),
        block: &crate::BlockKey,
    ) -> bool {
        let registry_keys = Rc::clone(&self.registry_keys);
        let keys = registry_keys.borrow();
        let Some(id) = self.capability_key_target(
            name,
            "block",
            &keys.blocks,
            mod_vocab::__private::parts_installation::block_key_slot(*block),
        ) else {
            return false;
        };
        self.note_capability(ProvidedBy::Block, name, binding)
            .is_some()
            && self.block_capability_covers(name, id)
    }

    fn entity_capability_registered(
        &mut self,
        name: &'static str,
        binding: (TypeId, &'static str),
        kind: &crate::EntityTypeKey,
    ) -> bool {
        let registry_keys = Rc::clone(&self.registry_keys);
        let keys = registry_keys.borrow();
        let Some(id) = self.capability_key_target(
            name,
            "entity type",
            &keys.entity_types,
            mod_vocab::__private::parts_installation::entity_type_key_slot(*kind),
        ) else {
            return false;
        };
        if self
            .note_capability(ProvidedBy::Entity, name, binding)
            .is_none()
        {
            return false;
        }
        self.parts
            .entity_capabilities
            .get(name)
            .and_then(|types| types.get(id))
            .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Entity,
                name,
                id,
            )
    }

    fn item_capability_registered(
        &mut self,
        name: &'static str,
        binding: (TypeId, &'static str),
        item: &crate::ItemKey,
    ) -> bool {
        let registry_keys = Rc::clone(&self.registry_keys);
        let keys = registry_keys.borrow();
        let Some(id) = self.capability_key_target(
            name,
            "item",
            &keys.items,
            mod_vocab::__private::parts_installation::item_key_slot(*item),
        ) else {
            return false;
        };
        if self
            .note_capability(ProvidedBy::Item, name, binding)
            .is_none()
        {
            return false;
        }
        self.parts
            .item_capabilities
            .get(name)
            .and_then(|types| types.get(id))
            .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Item,
                name,
                id,
            )
            || (name == crate::ITEM_ITEMS.name
                && self.parts.default_item_containers.contains_key(id))
    }

    fn set_block_capability_proxyable(
        &mut self,
        name: &'static str,
        binding: (TypeId, &'static str),
        proxyable: bool,
    ) {
        let Some(known) = self.note_capability(ProvidedBy::Block, name, binding) else {
            return;
        };
        if known.proxyable.is_some_and(|known| known != proxyable) {
            self.refused.push(format!("mod {} marked capability {name} proxyable={proxyable}, contradicting its earlier explicit mark", self.mod_id));
            return;
        }
        known.proxyable = Some(proxyable);
    }

    fn block_capability_names(&self) -> Vec<String> {
        self.parts
            .block_capability_order
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn block_capability_proxyable(&self, name: &str) -> bool {
        self.parts
            .block_capability_metadata
            .get(name)
            .is_some_and(|known| known.proxyable == Some(true))
    }
}
