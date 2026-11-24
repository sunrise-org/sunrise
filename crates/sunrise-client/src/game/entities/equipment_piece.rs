//! Shared eligibility and material resolution for worn humanoid equipment.
use super::*;
use sunrise_client::mobs::{
    finalize::EquipmentRead,
    render::{ArmorSlot, EquipmentModel},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EquipmentPass {
    Draw,
    TextureInventory,
}

pub(super) struct EquipmentPiece<'a> {
    pub asset: String,
    pub slot: ArmorSlot,
    pub kind: &'static str,
    pub layers: &'a [EquipmentAssetLayer],
    pub overlay: u32,
}

impl Game<'_> {
    pub(super) fn equipment_piece<'a>(
        &'a self,
        stack: &EquipmentRead<'_>,
        slot: &str,
        leggings: bool,
        baby: bool,
    ) -> Option<EquipmentPiece<'a>> {
        let base = stack.base();
        let equippable = sunrise_client::item_sync::equippable(base, self.res.items())?;
        let glint = self
            .res
            .items()
            .enchantments
            .props
            .get(&base.item)
            .and_then(|p| p.glint);
        let overlay = if stack.has_foil(glint) {
            sunrise_client::mobs::render::ARMOR_GLINT
        } else {
            0
        };
        resolve_piece(
            equippable,
            slot,
            leggings,
            baby,
            &self.armor_stand_assets.equipment,
            overlay,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EquipmentAssetLayer {
    pub texture: String,
    pub dyeable: bool,
    pub use_player_texture: bool,
    pub color_when_undyed: Option<u32>,
}
pub(super) type EquipmentAssets = HashMap<String, HashMap<String, Vec<EquipmentAssetLayer>>>;

fn resolve_piece<'a>(
    equippable: sunrise_client::item_sync::Equippable,
    slot: &str,
    leggings: bool,
    baby: bool,
    equipment: &'a EquipmentAssets,
    overlay: u32,
) -> Option<EquipmentPiece<'a>> {
    if equippable.slot != slot {
        return None;
    }
    let asset = equippable.asset_id?;
    let kind = if baby {
        "humanoid_baby"
    } else if leggings {
        "humanoid_leggings"
    } else {
        "humanoid"
    };
    let layers = equipment.get(&asset)?.get(kind)?;
    if layers.is_empty() {
        return None;
    }
    let slot = match slot {
        "head" => ArmorSlot::Head,
        "chest" => ArmorSlot::Chest,
        "legs" => ArmorSlot::Legs,
        "feet" => ArmorSlot::Feet,
        _ => return None,
    };
    Some(EquipmentPiece {
        asset,
        slot,
        kind,
        layers,
        overlay,
    })
}

impl Game<'_> {
    pub(super) fn mob_equipment_layers(
        &self,
        mob: &Mob,
        pass: EquipmentPass,
    ) -> Vec<sunrise_client::mobs::render::Layer> {
        use sunrise_client::mobs::{finalize, render};
        use sunrise_core::equipment_slot::EquipmentSlot::{Chest, Feet, Head, Legs};
        if sunrise_items::equipment::ARMOR
            .iter()
            .all(|&slot| mob.extra.slot_stack(slot).is_none())
            || render::avatar(mob)
            || sunrise_client::mobs::equine::is_equine(mob.species())
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (slot, layer, worn) in [
            (Chest, "chestplate", "chest"),
            (Legs, "leggings", "legs"),
            (Feet, "boots", "feet"),
            (Head, "helmet", "head"),
        ] {
            let Some(read) = finalize::equipment_slot(mob, slot) else {
                continue;
            };
            let Some(piece) = self.equipment_piece(&read, worn, layer == "leggings", mob.is_baby())
            else {
                continue;
            };
            let Some(key) = render::equipment_model_key(mob, layer) else {
                continue;
            };
            // The mob renderer resolves CEM textures before applying their emissive overlay.
            // Do not submit the default texture's glow a second time here.
            for material in self
                .equipment_materials(&piece, &read, pass)
                .into_iter()
                .filter(|m| !m.glows)
            {
                let mut layer = render::Layer::equipment(
                    &key,
                    material.binding.name,
                    material.color,
                    material.overlay,
                );
                layer.equipment_model = material.model.map(|model| (model.clone(), piece.slot));
                layer.equipment_uv = material.final_texture.then_some(material.binding.uv);
                out.push(layer);
            }
        }
        out
    }
}

#[derive(Clone, Copy)]
pub(super) struct MaterialBinding<'a> {
    pub name: &'a str,
    pub uv: [f32; 4],
}
pub(super) struct EquipmentMaterial<'a> {
    pub binding: MaterialBinding<'a>,
    pub color: u32,
    pub overlay: u32,
    pub glows: bool,
    pub model: Option<&'a Arc<EquipmentModel>>,
    pub final_texture: bool,
}

pub(super) struct ResolvedEquipmentLayer<'a> {
    pub binding: MaterialBinding<'a>,
    pub color: u32,
    pub model: Option<&'a Arc<EquipmentModel>>,
    pub final_texture: bool,
}

impl Game<'_> {
    pub(super) fn equipment_materials<'a>(
        &'a self,
        piece: &EquipmentPiece<'_>,
        read: &EquipmentRead<'_>,
        pass: EquipmentPass,
    ) -> Vec<EquipmentMaterial<'a>> {
        let stack = read.base();
        let sprites = &self.gui.atlas.sprites;
        let bind = |name: &str| {
            sprites
                .get_key_value(name)
                .map(|(name, r)| MaterialBinding {
                    name,
                    uv: [r.u0, r.v0, r.u1, r.v1],
                })
        };
        let cit = self.cit_worn_rule(sunrise_optifine::cit::Kind::Armor, stack);
        let trim = read
            .trim()
            .and_then(|trim| {
                self.armor_stand_assets
                    .trim_texture(&piece.asset, piece.kind, trim)
            })
            .and_then(|t| bind(&t.sprite()));
        let emissive = self
            .res
            .mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let texture = |name: &str| {
            sunrise_client::cit_armor::worn_layer(cit, &piece.asset, piece.kind, name, bind)
        };
        let glow = |name: &str| emissive.overlay(name).and_then(bind);
        if pass == EquipmentPass::Draw
            && let Some(materials) =
                self.extended_equipment_materials(piece, read, trim, texture, glow)
        {
            return materials;
        }
        piece.materials(Some(stack), trim, texture, glow)
    }
}

impl EquipmentPiece<'_> {
    // Only resolved atlas bindings reach submissions. Missing textures do not consume foil.
    fn materials<'a>(
        &self,
        stack: Option<&ItemStack>,
        trim: Option<MaterialBinding<'a>>,
        mut texture: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
        emissive: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
    ) -> Vec<EquipmentMaterial<'a>> {
        self.resolved_materials(
            trim,
            |_, layer| {
                let color =
                    super::armor_stands::layer_color(layer.dyeable, layer.color_when_undyed, stack);
                if color == 0 {
                    return None;
                }
                Some(ResolvedEquipmentLayer {
                    binding: texture(&layer.texture)?,
                    color,
                    model: None,
                    final_texture: false,
                })
            },
            emissive,
        )
    }

    pub(super) fn resolved_materials<'a>(
        &self,
        trim: Option<MaterialBinding<'a>>,
        mut resolve: impl FnMut(usize, &EquipmentAssetLayer) -> Option<ResolvedEquipmentLayer<'a>>,
        mut emissive: impl FnMut(&str) -> Option<MaterialBinding<'a>>,
    ) -> Vec<EquipmentMaterial<'a>> {
        let trim = trim.filter(|_| self.kind != "humanoid_baby");
        let mut foil = if trim.is_some() { 0 } else { self.overlay };
        let mut last_model = None;
        let mut out = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            let Some(ResolvedEquipmentLayer {
                binding,
                color,
                model,
                final_texture,
            }) = resolve(index, layer)
            else {
                continue;
            };
            last_model = model;
            let name = if final_texture {
                binding.name
            } else {
                &layer.texture
            };
            out.push(EquipmentMaterial {
                binding,
                color,
                overlay: foil,
                glows: false,
                model,
                final_texture,
            });
            foil = 0;
            if let Some(binding) = emissive(name) {
                out.push(EquipmentMaterial {
                    binding,
                    color,
                    overlay: 0,
                    glows: true,
                    model,
                    final_texture,
                });
            }
        }
        if let Some(binding) = trim {
            out.push(EquipmentMaterial {
                binding,
                color: 0xFFFF_FFFF,
                overlay: self.overlay,
                glows: false,
                model: last_model,
                final_texture: false,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::{
        item_sync::Equippable,
        mobs::{Part, anim::Poses, render},
    };

    pub(super) fn asset_layer(
        texture: &str,
        dyeable: bool,
        color_when_undyed: Option<u32>,
    ) -> EquipmentAssetLayer {
        EquipmentAssetLayer {
            texture: texture.into(),
            dyeable,
            use_player_texture: false,
            color_when_undyed,
        }
    }

    fn equippable(slot: &str, asset: Option<&str>) -> Equippable {
        Equippable {
            slot: slot.into(),
            asset_id: asset.map(str::to_owned),
            camera_overlay: None,
        }
    }

    #[test]
    fn asset_slot_and_baby_layer_gate() {
        let layers = vec![
            asset_layer("test:outer", false, None),
            asset_layer("test:inner", true, Some(0xA06540)),
        ];
        let equipment = HashMap::from([(
            "test:chest".into(),
            HashMap::from([
                ("humanoid".into(), layers.clone()),
                ("humanoid_leggings".into(), vec![]),
                (
                    "humanoid_baby".into(),
                    vec![asset_layer("test:baby", false, None)],
                ),
            ]),
        )]);
        let resolve = |slot, leggings, baby| {
            resolve_piece(
                equippable("chest", Some("test:chest")),
                slot,
                leggings,
                baby,
                &equipment,
                36,
            )
        };
        assert!(resolve("head", false, false).is_none());
        assert!(resolve("chest", true, false).is_none());
        assert!(
            resolve_piece(
                equippable("chest", None),
                "chest",
                false,
                false,
                &equipment,
                0
            )
            .is_none()
        );
        assert!(
            resolve_piece(
                equippable("chest", Some("test:unknown")),
                "chest",
                false,
                false,
                &equipment,
                0
            )
            .is_none()
        );
        assert_eq!(resolve("chest", false, false).unwrap().layers, layers);
        assert_eq!(
            resolve("chest", true, true).unwrap().layers[0].texture,
            "test:baby"
        );
    }

    // Pin the actual vertex frame, not only material names: the player and stand
    // still submit the same model, UV, colour and foil after the shared gate.
    #[test]
    fn player_and_stand_material_frames_are_identical() {
        let model = Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![sunrise_client::mobs::Polygon {
                normal: [0.0, 0.0, 1.0],
                vertices: [
                    [0.0, 0.0, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0, 0.0, 1.0],
                ],
                cull_back: false,
            }],
            children: Vec::new(),
            layer: None,
        };
        let poses = Poses::new(&model);
        let equipment = HashMap::from([(
            "test:leather".into(),
            HashMap::from([(
                "humanoid".into(),
                vec![
                    asset_layer("test:base", true, Some(0xA06540)),
                    asset_layer("test:overlay", false, None),
                ],
            )]),
        )]);
        let piece = resolve_piece(
            equippable("chest", Some("test:leather")),
            "chest",
            false,
            false,
            &equipment,
            0,
        )
        .unwrap();
        for base in [
            glam::Mat4::IDENTITY,
            glam::Mat4::from_scale(glam::Vec3::splat(0.5)),
        ] {
            let mut expected = Vec::new();
            let mut actual = Vec::new();
            for (uv, tint, foil) in [
                ([0.0, 0.0, 0.5, 0.5], 0xFF4065A0, 0),
                ([0.5, 0.5, 1.0, 1.0], 0xFFFFFFFF, 0),
            ] {
                render::draw(&model, base, &poses, uv, tint, 0xF0F0, foil, &mut expected);
            }
            for material in piece.materials(
                None,
                None,
                |name| match name {
                    "test:base" => Some(MaterialBinding {
                        name: "test:base",
                        uv: [0.0, 0.0, 0.5, 0.5],
                    }),
                    "test:overlay" => Some(MaterialBinding {
                        name: "test:overlay",
                        uv: [0.5, 0.5, 1.0, 1.0],
                    }),
                    _ => None,
                },
                |_| None,
            ) {
                render::draw(
                    &model,
                    base,
                    &poses,
                    material.binding.uv,
                    material.color,
                    0xF0F0,
                    material.overlay,
                    &mut actual,
                );
            }
            assert!(!actual.is_empty());
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&actual),
                bytemuck::cast_slice::<_, u8>(&expected)
            );
        }
    }
}

#[cfg(test)]
mod material_tests {
    use super::tests::asset_layer;
    use super::*;
    #[test]
    fn dyeable_without_default_uses_dye_or_skips_before_foil() {
        let layers = vec![
            asset_layer("dyed_only", true, None),
            asset_layer("outer", false, None),
        ];
        let piece = EquipmentPiece {
            asset: "test:dyed".into(),
            slot: ArmorSlot::Chest,
            kind: "humanoid",
            layers: &layers,
            overlay: 36,
        };
        let mut stack = ItemStack::new("test:equipment", 1);
        let draw = |stack: &ItemStack| {
            piece
                .materials(
                    Some(stack),
                    None,
                    |name| {
                        let name = match name {
                            "dyed_only" => "dyed_only",
                            "outer" => "outer",
                            _ => return None,
                        };
                        Some(MaterialBinding {
                            name,
                            uv: [0.0, 0.0, 1.0, 1.0],
                        })
                    },
                    |_| None,
                )
                .into_iter()
                .map(|m| (m.binding.name, m.color, m.overlay))
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(&stack), [("outer", 0xFFFFFFFF, 36)]);
        stack.data_mut().dyed_color = Some(0x123456);
        assert_eq!(
            draw(&stack),
            [("dyed_only", 0xFF563412, 36), ("outer", 0xFFFFFFFF, 0)]
        );
    }

    #[test]
    fn ordered_materials_first_drawn_foil_trim_and_babies() {
        let layers = vec![
            asset_layer("missing", false, None),
            asset_layer("inner", false, None),
            asset_layer("outer", false, None),
        ];
        let mut piece = EquipmentPiece {
            asset: "test:custom".into(),
            slot: ArmorSlot::Chest,
            kind: "humanoid",
            layers: &layers,
            overlay: 36,
        };
        let stack = ItemStack::new("test:not_an_armor_name", 1);
        let draw = |piece: &EquipmentPiece<'_>, trim: Option<&'static str>| {
            let bind = |name| MaterialBinding {
                name,
                uv: [0.0, 0.0, 1.0, 1.0],
            };
            piece
                .materials(
                    Some(&stack),
                    trim.map(bind),
                    |name| match name {
                        "inner" => Some(bind("inner")),
                        "outer" => Some(bind("outer")),
                        _ => None,
                    },
                    |_| None,
                )
                .into_iter()
                .map(|l| (l.binding.name.to_owned(), l.overlay))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            draw(&piece, None),
            [("inner".into(), 36), ("outer".into(), 0)]
        );
        assert_eq!(
            draw(&piece, Some("trim")),
            [
                ("inner".into(), 0),
                ("outer".into(), 0),
                ("trim".into(), 36)
            ]
        );
        piece.kind = "humanoid_baby";
        assert_eq!(
            draw(&piece, Some("trim")),
            [("inner".into(), 36), ("outer".into(), 0)]
        );
    }
}
