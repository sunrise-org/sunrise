//! Block entities a moving piston carries (#3184): taken out of the world with
//! their attachments and without removal side effects, held by the moving
//! piston's position, and installed where their block lands as a new holder.
//! Each has one owner at a time: it is moved, never cloned, so what it holds
//! can't be duplicated.
use super::{BlockEntities, BlockEntity, Pos, chunk_of, identity};
use std::collections::hash_map::Entry;
use sunrise_mod_runtime::attachments::Attachments;

/// A block entity out of the world while a piston moves its block.
pub struct Carried {
    pub entity: BlockEntity,
    /// Position-owned saved fields and adapter state, owned by the carried instance.
    pub position_data: sunrise_core::nbt::Compound,
    /// Its attachment map, when it had one.
    pub attachments: Option<Attachments>,
}

impl BlockEntities {
    /// Takes the block entity at `from` out of the world for the moving piston
    /// at `to`. Its slot is freed and the attachment index sees the holder
    /// leave, but it is not removed: no side effects run, so nothing it holds
    /// drops. False when there is none, or it is out for its own tick.
    pub fn carry(&mut self, from: Pos, to: Pos) -> bool {
        let Some(owned) = self.extract_owned(from) else {
            return false;
        };
        self.hold_carried(to, owned);
        true
    }

    /// Relinquish the actual payload without removal callbacks or inventory cloning.
    pub fn extract_owned(&mut self, from: Pos) -> Option<Carried> {
        if !self.map.contains_key(&from) || self.is_taken(&from) {
            return None;
        }
        let attachments = self.attachments.remove(&from);
        if attachments.is_some()
            && let Some(observer) = &self.attachment_observer
        {
            observer.unload(identity(from, &self.generations));
        }
        let entity = self
            .remove(&from)
            .expect("owned block entity remained in its slot");
        Some(Carried {
            entity,
            position_data: Default::default(),
            attachments,
        })
    }

    /// Install unique ownership in an empty holder; refusal returns the unchanged owned payload.
    pub fn install_owned(
        &mut self,
        at: Pos,
        owned: Carried,
    ) -> Result<sunrise_core::nbt::Compound, Box<Carried>> {
        if self.map.contains_key(&at) || self.is_taken(&at) {
            return Err(Box::new(owned));
        }
        let Carried {
            entity,
            position_data,
            attachments,
        } = owned;
        self.insert(at, entity);
        if let Some(attachments) = attachments {
            self.restore_attachments(at, attachments);
        }
        Ok(position_data)
    }

    /// Install the moving piston's payload as a new ordinary holder.
    pub fn land(&mut self, at: Pos) -> bool {
        let Some(owned) = self.take_carried(&at) else {
            return false;
        };
        // Preserve the existing piston replacement rule. Checked general handoffs call
        // install_owned directly and must admit an empty destination before relinquishing source.
        self.remove(&at);
        match self.install_owned(at, owned) {
            Ok(_) => true,
            Err(owned) => {
                self.hold_carried(at, *owned);
                false
            }
        }
    }
    /// The block entity carried to `at`, for its moving piston's save.
    pub fn carried(&self, at: &Pos) -> Option<&Carried> {
        self.carried.get(&chunk_of(*at))?.get(at)
    }
    /// An in-flight tick owns the payload: a piston must refuse rather than replace its lease.
    pub fn is_taken(&self, at: &Pos) -> bool {
        self.out.contains_key(at)
    }
    /// The owning adapter updates position-owned state before a save or landing.
    pub fn carried_mut(&mut self, at: &Pos) -> Option<&mut Carried> {
        self.touch_contents();
        self.carried.get_mut(&chunk_of(*at))?.get_mut(at)
    }
    /// Holds `carried` for the moving piston at `at`, loaded with it.
    pub fn hold_carried(&mut self, at: Pos, carried: Carried) {
        self.touch_contents();
        self.carried
            .entry(chunk_of(at))
            .or_default()
            .insert(at, carried);
    }
    /// Drops the block entity carried to `at` without installing it: a
    /// client's copy once the server's block took the moving piston's place.
    pub fn discard_carried(&mut self, at: &Pos) -> bool {
        self.take_carried(at).is_some()
    }

    fn take_carried(&mut self, at: &Pos) -> Option<Carried> {
        self.touch_contents();
        let Entry::Occupied(mut column) = self.carried.entry(chunk_of(*at)) else {
            return None;
        };
        let payload = column.get_mut().remove(at);
        if column.get().is_empty() {
            column.remove();
        }
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BlockEntityIdentity, Ticking};
    use super::*;
    use std::sync::{Arc, Mutex};
    use sunrise_mod_runtime::attachments::{
        AttachmentType, Holder, index::AttachmentIndex, observer::IndexedAttachmentObserver,
    };

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "carry_test",
        name: "value",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };

    #[test]
    fn a_carried_block_entity_leaves_its_holder_and_lands_as_a_new_one() {
        let index = Arc::new(Mutex::new(AttachmentIndex::<BlockEntityIdentity>::default()));
        let mut entities = BlockEntities::default();
        entities.observe_attachments(Arc::new(IndexedAttachmentObserver::new(&index, Some)));
        entities.set_states(vec![Ticking::Yes]);
        let (from, to) = ((1, 64, 1), (2, 64, 1));
        entities.insert(from, BlockEntity::Chest(vec![None, None, None]));
        VALUE
            .set(entities.attachments_mut(from).unwrap(), Holder::Level, 7)
            .unwrap();
        let rows = || {
            index
                .lock()
                .unwrap()
                .snapshot(&("carry_test".into(), "value".into()))
                .iter()
                .map(|row| (row.holder.position, *row.entry.read(&VALUE).unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(entities.tickers(|_| 0).len(), 1);

        assert!(entities.carry(from, to));
        assert!(!entities.contains_key(&from) && !entities.contains_key(&to));
        assert!(entities.attachments(from).is_none());
        assert!(rows().is_empty());
        assert!(entities.tickers(|_| 0).is_empty());

        assert!(entities.land(to));
        assert!(matches!(entities.get(&to), Some(BlockEntity::Chest(slots)) if slots.len() == 3));
        assert_eq!(rows(), [([2, 64, 1], 7)]);
        assert_eq!(
            entities
                .tickers(|_| 0)
                .iter()
                .map(|(p, _)| *p)
                .collect::<Vec<_>>(),
            [to]
        );
        assert!(!entities.land(to));
    }

    #[test]
    fn unloading_its_chunk_drops_a_carried_block_entity() {
        let mut entities = BlockEntities::default();
        entities.insert((1, 64, 1), BlockEntity::Chest(vec![]));
        assert!(entities.carry((1, 64, 1), (17, 64, 1)));
        entities.unload(|p| p.0 >> 4 == 1);
        assert!(entities.carried(&(17, 64, 1)).is_none());
        assert!(!entities.land((17, 64, 1)));
    }

    #[test]
    fn unloading_columns_drops_only_their_destined_payloads() {
        let mut entities = BlockEntities::default();
        let (west, east, retained) = ((-1, 64, 1), (16, 64, 1), (0, 64, 2));
        entities.insert((0, 64, 1), BlockEntity::Chest(vec![]));
        assert!(entities.carry((0, 64, 1), west));
        entities.insert((15, 64, 1), BlockEntity::Chest(vec![]));
        assert!(entities.carry((15, 64, 1), east));
        entities.insert((-1, 64, 2), BlockEntity::Chest(vec![None; 3]));
        assert!(entities.carry((-1, 64, 2), retained));

        entities.insert((17, 64, 1), BlockEntity::Chest(vec![]));
        for column in [(-1, 0), (1, 0), (2, 0)] {
            entities.unload_column(column);
        }

        assert_eq!(entities.take_column_unload_entries(), 3);
        assert!(!entities.contains_key(&(17, 64, 1)));
        assert!(!entities.land(west));
        assert!(!entities.land(east));
        assert!(entities.land(retained));
        assert!(
            matches!(entities.get(&retained), Some(BlockEntity::Chest(slots)) if slots.len() == 3)
        );
    }
}

#[cfg(test)]
#[path = "owned_transfer_tests.rs"]
mod owned_transfer_tests;
