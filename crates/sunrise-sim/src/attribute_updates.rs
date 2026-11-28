//! Resident ServerPlayer pending attribute callbacks over its owned live map.
//! Waypoint requests are observable boundaries; manager/network delivery is
//! still supplied by the eventual multiplayer runtime.
use crate::{
    Result,
    health::Updates,
    motion::{MotionEvent, PlayerMotion},
};
use std::io;
use sunrise_world::{
    attribute_map::ValueKind,
    attribute_updates::{self, UpdateWorld, UpdatedAttribute},
    attributes::live::{self, Handle, Key, LiveAttributes},
    live_attribute_updates::{self as live_updates, LiveAttributeWorld},
    living_health::{self, HealthWorld},
    object_open_set::CursorError,
};

impl PlayerMotion {
    /// LivingEntity.tick refreshes these callbacks after aiStep and before the
    /// enclosing Player.tick selects its pose. Synchronization stays queued.
    pub fn refresh_attributes(&mut self) -> Result<()> {
        live_updates::refresh(self)
    }
    /// `MobEffect.addAttributeModifiers` or `removeAttributeModifiers` on the player.
    pub fn write_effect_modifiers(&mut self, index: Option<usize>, amplifier: Option<i32>) {
        use sunrise_world::effect_attribute_modifiers::{Definitions, Template, add_definition};
        let Some(effect) = index.and_then(sunrise_game_data::potions::effect_by_index) else {
            return;
        };
        let mut definitions = Definitions::default();
        for (attribute, id, amount, operation) in crate::effects::effect_modifiers(effect) {
            if let Some(key) = self.attributes.key(attribute)
                && let Ok(template) = Template::new(id, amount, operation)
            {
                add_definition(&mut definitions, Some(key), template);
            }
        }
        let result = match amplifier {
            Some(amplifier) => self
                .attributes
                .add_effect_modifiers(&mut definitions, amplifier),
            None => self.attributes.remove_effect_modifiers(&mut definitions),
        };
        if let Err(error) = result {
            log::error!(
                "Could not update player attribute modifiers for {}: {error:?}",
                effect.id
            );
        }
    }
    /// `Player.internalSetAbsorptionAmount`: the amount as given, unclamped
    /// (the saved `AbsorptionAmount`, or what a client's server sent).
    pub fn internal_set_absorption(&mut self, value: f32) {
        self.store_absorption(value)
            .expect("absorption float bound at resource load");
    }
}
impl HealthWorld for PlayerMotion {
    type Error = Box<dyn std::error::Error>;
    type Data = ();
    fn health(&mut self) -> Result<f32> {
        Ok(self.health.health())
    }
    fn health_data(&self) {}
    fn max_health_attribute(&mut self) -> Result<f64> {
        self.attribute("minecraft:max_health")
    }
    fn max_absorption_attribute(&mut self) -> Result<f64> {
        self.attribute("minecraft:max_absorption")
    }
    fn store_health(&mut self, _: (), value: f32) -> Result<()> {
        self.health.data.set(
            self.health.ids.health,
            value,
            false,
            &mut Updates(&mut self.events),
        )?;
        Ok(())
    }
    fn store_absorption(&mut self, value: f32) -> Result<()> {
        // Player.internalSetAbsorptionAmount stores the already clamped value
        // through its current synchronized data, without another limit query.
        self.health.data.set(
            self.health.ids.absorption,
            value,
            false,
            &mut Updates(&mut self.events),
        )?;
        Ok(())
    }
    fn dead_flag(&self) -> bool {
        self.health.dead
    }
    fn dispatch_set_health(&mut self, value: f32) -> Result<()> {
        living_health::set_health(self, value)
    }
}
impl UpdateWorld<Key> for PlayerMotion {
    type Level = ();
    type Manager = ();
    type Attributes = ();
    fn attribute_kind(&self, attribute: &Key) -> UpdatedAttribute {
        live_updates::kind(attribute)
    }
    fn absorption(&mut self) -> Result<f32> {
        Ok(self.health.absorption())
    }
    fn refresh_dimensions(&mut self) -> Result<()> {
        PlayerMotion::refresh_dimensions(self)
    }
    fn level(&mut self) -> Result<()> {
        Ok(())
    }
    fn is_server_level(&self, _: ()) -> bool {
        true
    }
    fn waypoint_manager(&mut self, _: ()) -> Result<()> {
        Ok(())
    }
    fn attributes_field(&self) {}
    fn attribute_value_in(&mut self, _: (), attribute: Key) -> Result<f64> {
        self.attributes
            .value(Some(attribute), ValueKind::Computed)
            .map_err(Self::attribute_error)
    }
    fn track_waypoint(&mut self, _: (), track: bool) -> Result<()> {
        self.events.push(MotionEvent::Waypoint {
            receiver: false,
            enabled: track,
        });
        Ok(())
    }
}
impl LiveAttributeWorld for PlayerMotion {
    type Identity = Box<dyn sunrise_world::attributes::live::Identities>;
    fn get_attributes(&mut self) -> Result<()> {
        Ok(())
    }
    fn live_attributes(&self, _: ()) -> &LiveAttributes<Self::Identity> {
        &self.attributes
    }
    fn live_attributes_mut(&mut self, _: ()) -> &mut LiveAttributes<Self::Identity> {
        &mut self.attributes
    }
    fn instance_attribute(&self, instance: Option<Handle>) -> Result<Key> {
        let instance =
            instance.ok_or_else(|| io::Error::other("null pending attribute instance"))?;
        self.attributes
            .attribute(&instance)
            .cloned()
            .map_err(Self::attribute_error)
    }
    fn attribute_error(error: live::Error) -> Self::Error {
        io::Error::other(format!("resident attribute: {error:?}")).into()
    }
    fn cursor_error(error: CursorError) -> Self::Error {
        io::Error::other(format!("resident attribute cursor: {error:?}")).into()
    }
    fn dispatch_attribute_updated(&mut self, attribute: Key) -> Result<()> {
        // ServerPlayer.onAttributeUpdated handles receive range, then calls
        // the inherited LivingEntity handler, including for this same holder.
        if attribute.name() == "minecraft:waypoint_receive_range" {
            let value = self
                .attributes
                .value(Some(attribute.clone()), ValueKind::Computed)
                .map_err(Self::attribute_error)?;
            self.events.push(MotionEvent::Waypoint {
                receiver: true,
                enabled: value > 0.0,
            });
        }
        attribute_updates::updated(self, attribute)
    }
}
