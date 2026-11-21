use super::*;
use sunrise_client::mobs::{anim, render};

impl render::RenderDataSource for Game<'_> {
    fn mob_data(&self) -> &sunrise_client::mobs::MobData {
        self.res.mob_data()
    }

    fn equipment_layers(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(mob, super::equipment_piece::EquipmentPass::Draw)
    }

    fn equipment_inventory(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(mob, super::equipment_piece::EquipmentPass::TextureInventory)
    }

    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        mod_api::__private::listened(mod_api::HookId::HumanoidSetupAnim)
            .then_some(self as &dyn anim::HumanoidSetup)
    }
}

impl anim::HumanoidSetup for Game<'_> {
    fn before_arm_pose(&self, mob: &Mob, state: &anim::State<'_>, poses: &mut anim::Poses) {
        use super::api_values::TryIntoApi;
        let stacks = [false, true].map(|off| {
            render::held_item_stack(&mob.extra, off).and_then(|stack| stack.try_into_api().ok())
        });
        let [main, off] = anim::humanoid_arm_poses(mob, mob.extra.held_arm_pose, state.swing);
        let humanoid = mod_api::HumanoidRenderState {
            main_hand: stacks[0].as_ref(),
            off_hand: stacks[1].as_ref(),
            main_arm_left: mob.main_arm_left(),
            is_using_item: mob.extra.using_item > 0,
            use_hand: (mob.extra.using_item > 0).then_some(if mob.extra.using_off_hand {
                Hand::Off
            } else {
                Hand::Main
            }),
            ticks_using_item: mob.extra.using_item,
            use_animation: mob.extra.use_animation,
            main_arm_pose: main.try_into_api().unwrap(),
            off_arm_pose: off.try_into_api().unwrap(),
        };
        self.setup_humanoid_parts(mob.id, &mob.kind.id, &humanoid, state.partial, poses);
    }
}

impl Game<'_> {
    /// One adapter for mob and player models. Parts have a finite
    /// value invariant, so applying the completed listener answer needs no second
    /// validation pass and cannot overwrite scale or visibility.
    pub(super) fn setup_humanoid_parts(
        &self,
        entity_id: u32,
        entity_type: &str,
        humanoid: &mod_api::HumanoidRenderState<'_>,
        partial_tick: f32,
        poses: &mut anim::Poses,
    ) {
        let values = mod_api::HumanoidParts::PARTS.map(|part| {
            let name = part.model_name();
            poses
                .pose_of(name)
                .map(|pose| [[pose[0], pose[1], pose[2]], [pose[3], pose[4], pose[5]]])
        });
        let Some(mut parts) = mod_api::HumanoidParts::new(values) else {
            return;
        };
        self.native_humanoid_setup_anim(mod_api::__private::HumanoidSetupAnimCall {
            entity_id,
            entity_type,
            humanoid,
            partial_tick,
            game_time: self.title.is_none().then_some(self.world.game_time),
            parts: &mut parts,
        });
        for (part, original) in mod_api::HumanoidParts::PARTS.into_iter().zip(values) {
            // A listener may replace its whole snapshot, but cannot add a part
            // this model never had (or write the engine's dummy pose).
            if original.is_some()
                && let Some([position, rotation]) = parts.get(part)
            {
                let target = poses.get(part.model_name());
                target[..3].copy_from_slice(&position);
                target[3..6].copy_from_slice(&rotation);
            }
        }
    }
}
