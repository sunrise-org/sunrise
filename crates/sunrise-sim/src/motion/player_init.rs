use super::*;

pub(super) fn load_player_attributes(
    root: &Path,
    finalize: impl FnOnce(Catalog) -> Result<Catalog>,
) -> Result<(Supplier, f32)> {
    let path = root.join("attributes.json");
    let prepared = Catalog::from_json(&sunrise_world::json_file::read(&path)?)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let catalog = Arc::new(finalize(prepared)?);
    let invalid = |cause: &str| format!("{}: {cause}", path.display());
    let max_health = catalog
        .id("minecraft:max_health")
        .ok_or_else(|| invalid("missing minecraft:max_health definition"))?;
    catalog
        .id("minecraft:max_absorption")
        .ok_or_else(|| invalid("missing minecraft:max_absorption definition"))?;
    let supplier = Supplier::bind(catalog.clone(), "minecraft:player")
        .ok_or_else(|| invalid("missing minecraft:player supplier"))?;
    let base = supplier
        .base(max_health)
        .ok_or_else(|| invalid("missing minecraft:max_health base for minecraft:player"))?;
    let maximum = catalog.attribute(max_health).definition.sanitize(base) as f32;
    Ok((supplier, maximum))
}

impl PlayerMotion {
    pub fn new(resources: &MotionResources, position: Vec3) -> Self {
        // Process-owned identities determine native collection callback order.
        let mut identity = 0i32;
        let next: Box<dyn sunrise_world::attributes::live::Identities> = Box::new(move || {
            identity = identity.wrapping_add(1);
            identity
        });
        Self::from_attributes(resources, position, resources.player_supplier.create(next))
    }

    /// Explicit identity allocation for deterministic reference replay.
    pub fn with_attribute_identities(
        resources: &MotionResources,
        position: Vec3,
        hashes: &[i32],
        next: Box<dyn sunrise_world::attributes::live::Identities>,
    ) -> Result<Self> {
        let attributes = resources
            .player_supplier
            .with_identities(hashes, next)
            .map_err(attribute_error)?;
        Ok(Self::from_attributes(resources, position, attributes))
    }

    fn from_attributes(
        resources: &MotionResources,
        position: Vec3,
        attributes: crate::living_attributes::Live,
    ) -> Self {
        Self {
            random: sunrise_core::random::EntityRandom::own(crate::mobs::entity_random(
                0, position,
            )),
            health: resources.initial_player_health.clone(),
            status: crate::effects::StatusFlags::default(),
            impulse: ImpulseContext::default(),
            spawn_extra_particles_on_fall: false,
            position,
            velocity: Vec3::ZERO,
            ground: GroundState::default(),
            collision: CollisionFlags::default(),
            history: MovementHistory::default(),
            fall_distance: 0.0,
            record_fall_reset: false,
            block_landing: None,
            stuck_multiplier: Vec3::ZERO,
            piston: Default::default(),
            yaw: 0.0,
            head_yaw: 0.0,
            body_yaw: 0.0,
            jumping: false,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            may_fly: false,
            mobility_restricted: false,
            invulnerable: false,
            spectator: false,
            local_spectator: false,
            flying: false,
            flying_speed: 0.05,
            was_jumping: false,
            auto_jump_enabled: false,
            auto_jump_time: 0,
            was_auto_jump: false,
            move_vector: [0.0, 0.0],
            applied_input: [0.0, 0.0],
            glider: false,
            pitch: 0.0,
            fall_flying: false,
            sleeping: false,
            fall_fly_ticks: 0,
            glide_ending: false,
            server_owns_glide: false,
            glider_wear: 0,
            location_effects: LocationEffects::default(),
            auto_spin_attack_ticks: 0,
            auto_spin_attack_dmg: 0.0,
            auto_spin_attack_item: None,
            spin_targets: Vec::new(),
            spin_hit: None,
            sprinting: false,
            sprint_input: crate::sprint::SprintState::default(),
            shift_key_down: false,
            local_crouching: false,
            pose: crate::pose::Pose::Standing,
            dimensions: crate::pose::Pose::Standing.dimensions(1.0),
            size_answer: None,
            pose_fit_sizes: [None; 3],
            scale: 1.0,
            swimming: false,
            is_passenger: false,
            fire_ticks: -20,
            mod_fire_immune: false,
            air: 300,
            respiration: 0,
            speed: 0.0,
            main_hand: None,
            item_use_speed: None,
            item_use_slows_sprint: false,
            nudge: [None; 2],
            last_climbable: None,
            on_climbable: false,
            steps: MovementDistances {
                next_step: 1.0,
                ..MovementDistances::default()
            },
            crystal_sound_intensity: 0.0,
            last_crystal_sound_play_tick: 0,
            needs_sync: false,
            sync_position: false,
            teleported: false,
            teleported_delta: [false; 3],
            events: Vec::new(),
            attributes,
            fluids: PlayerFluids::default(),
            in_powder_snow: false,
            was_in_powder_snow: false,
            ticks_frozen: 0,
            leather_boots: false,
            freeze_immune: false,
            old_position: position,
            server_copy: false,
            replaying: false,
            wall_hit_events: None,
            client_side: false,
            entity_id: 0,
            client_loaded: true,
            block_effects: true,
            hurt_log: Vec::new(),
            attack_log: Vec::new(),
            border: None,
            collision_exclusions: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn prepared_catalog(root: &Path, player: Value, attributes: &[&str]) {
        let attributes: Vec<_> = attributes
            .iter()
            .map(|id| json!({"id": id, "default": "0000000000000000", "syncable": true}))
            .collect();
        std::fs::write(
            root.join("attributes.json"),
            json!({
                "data_version": sunrise_core::version::DATA_VERSION,
                "attributes": attributes,
                "entities": player,
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn prepared_load_names_each_missing_player_requirement() {
        let root = tempfile::tempdir().unwrap();
        let health = "minecraft:max_health";
        let absorption = "minecraft:max_absorption";
        let complete_row =
            || json!({"minecraft:player": {"minecraft:max_health": "4034000000000000"}});
        for (entities, definitions, cause) in [
            (
                json!({}),
                vec![health, absorption],
                "missing minecraft:player supplier",
            ),
            (
                json!({"minecraft:player": {}}),
                vec![health, absorption],
                "missing minecraft:max_health base for minecraft:player",
            ),
            (
                json!({"minecraft:player": {}}),
                vec![absorption],
                "missing minecraft:max_health definition",
            ),
            (
                complete_row(),
                vec![health],
                "missing minecraft:max_absorption definition",
            ),
            (
                complete_row(),
                vec![absorption],
                "missing minecraft:max_health definition referenced by minecraft:player supplier",
            ),
        ] {
            prepared_catalog(root.path(), entities, &definitions);
            let error = load_player_attributes(root.path(), Ok).err().unwrap();
            assert_eq!(
                error.to_string(),
                format!("{}: {cause}", root.path().join("attributes.json").display())
            );
        }
    }

    #[test]
    fn prepared_load_binds_the_contributed_player_max_health() {
        let root = tempfile::tempdir().unwrap();
        prepared_catalog(
            root.path(),
            json!({"minecraft:player": {"minecraft:max_health": "4034000000000000"}}),
            &["minecraft:max_health", "minecraft:max_absorption"],
        );
        let (_, original) = load_player_attributes(root.path(), Ok).unwrap();
        assert_eq!(original, 20.0);
        let (supplier, changed) = load_player_attributes(root.path(), |catalog| {
            catalog
                .with_entity_additions(
                    "minecraft:player",
                    &[("minecraft:max_health".into(), Some(36.0))],
                )
                .map_err(|error| error.to_string().into())
        })
        .unwrap();
        assert_eq!(changed, 36.0);
        let live = supplier.create(|| 1);
        assert_eq!(
            live.value(live.key("minecraft:max_health"), ValueKind::Computed)
                .unwrap(),
            36.0
        );
    }
}
