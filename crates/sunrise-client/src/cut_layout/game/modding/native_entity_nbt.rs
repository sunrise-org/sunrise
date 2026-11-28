use crate::game::{
    client_side::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent},
    server_side::{
        native_entity_refs::tests::{run_fresh_cases, with_client_game},
        native_guest_turn_tests::with_resident_guests,
    },
};
use mod_api::{EntityRef, WorldEpoch};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::{
    commands_common::target_id, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
};
use sunrise_protocol::{
    entity_data::{PaintingHolder, PaintingVariant, Value, serializer as s},
    item::Stack,
    packets::play::{self, Clientbound as C},
};
use sunrise_sim::{entity_nbt, lan::playerdata::ints_uuid, save::doubles};

#[test]
#[ignore = "daily"]
fn replica_nbt_matches_synced_fields_and_keeps_client_defaults() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_entity_state::client_nbt_tests::replica_nbt_matches_synced_fields_and_keeps_client_defaults",
        "SUNRISE_TEST_REPLICA_NBT_CASE",
        &["replica"],
    ) {
        return;
    }

    const AT: [f64; 3] = [2.5, 65.0, 3.5];
    let stone = Tag::compound([
        ("id", Tag::String("minecraft:stone".into())),
        ("count", Tag::Int(1)),
    ]);
    let name = Tag::String("Fixture".into());
    let block = Tag::IntArray(vec![2, 65, 3]);
    let fixtures: [(&str, Vec<(&str, Tag)>, &[&str], i32); 8] = [
        (
            "minecraft:armor_stand",
            vec![("ShowArms", Tag::Byte(1)), ("DisabledSlots", Tag::Int(7))],
            &["ShowArms", "Health", "Small", "NoBasePlate", "Pose"],
            0,
        ),
        (
            "minecraft:item_frame",
            vec![
                ("Item", stone.clone()),
                ("block_pos", block.clone()),
                ("Facing", Tag::Byte(2)),
                ("ItemRotation", Tag::Byte(3)),
                ("ItemDropChance", Tag::Float(0.5_f32.to_bits())),
                ("Fixed", Tag::Byte(1)),
            ],
            &["Item", "block_pos", "Facing", "ItemRotation", "Invisible"],
            2,
        ),
        (
            "minecraft:painting",
            vec![
                ("block_pos", block),
                ("facing", Tag::Byte(2)),
                ("variant", Tag::String("minecraft:kebab".into())),
            ],
            &["block_pos", "facing", "variant"],
            2,
        ),
        (
            "minecraft:item",
            vec![
                ("Item", stone),
                ("Age", Tag::Short(42)),
                ("PickupDelay", Tag::Short(20)),
            ],
            &["Item"],
            0,
        ),
        (
            "minecraft:text_display",
            vec![
                ("text", Tag::String("Replica text".into())),
                ("line_width", Tag::Int(84)),
                ("width", Tag::Float(1.25_f32.to_bits())),
                ("start_interpolation", Tag::Int(16)),
            ],
            &[
                "text",
                "line_width",
                "width",
                "transformation",
                "billboard",
                "brightness",
                "interpolation_duration",
                "teleport_duration",
                "view_range",
                "shadow_radius",
                "shadow_strength",
                "height",
                "glow_color_override",
                "background",
                "text_opacity",
                "shadow",
                "see_through",
                "default_background",
                "alignment",
            ],
            0,
        ),
        ("minecraft:oak_boat", vec![], &["Pos", "Motion"], 0),
        (
            "minecraft:cow",
            vec![
                ("Age", Tag::Int(60)),
                ("Health", Tag::Float(7.0_f32.to_bits())),
                ("AgeLocked", Tag::Byte(1)),
                ("PersistenceRequired", Tag::Byte(1)),
            ],
            &["Health", "AgeLocked"],
            0,
        ),
        (
            "minecraft:area_effect_cloud",
            vec![
                ("Radius", Tag::Float(2.5_f32.to_bits())),
                ("Duration", Tag::Int(40)),
            ],
            &["Radius"],
            0,
        ),
    ];
    let mut authoritative = Vec::new();
    with_resident_guests(|server| {
        for (i, (kind, extra, _, _)) in fixtures.iter().enumerate() {
            let tag = Tag::compound(
                [
                    ("id", Tag::String((*kind).into())),
                    ("UUID", Tag::IntArray(vec![1, 2, 3, i as i32 + 10])),
                    ("Pos", doubles(&AT)),
                    ("Air", Tag::Short(145)),
                    ("CustomName", name.clone()),
                    ("CustomNameVisible", Tag::Byte(1)),
                    ("Silent", Tag::Byte(1)),
                    ("NoGravity", Tag::Byte(1)),
                    ("TicksFrozen", Tag::Int(25)),
                    ("Fire", Tag::Short(50)),
                    ("Glowing", Tag::Byte(1)),
                ]
                .into_iter()
                .chain(extra.iter().cloned()),
            );
            let target = server
                .spawn_entity_from_nbt(tag)
                .unwrap_or_else(|| panic!("spawn {kind}"));
            let entity = server.entity(target_id(target).unwrap()).unwrap();
            let fields = entity.nbt(server).unwrap();
            assert!(!fields.contains_key(&"id".into()));
            authoritative.push(Tag::Compound(fields));
        }
    });

    with_client_game(|client| {
        let mut reader = logged_in(client);
        let builtin = client.client.remote.as_ref().unwrap().builtin.clone();
        let kind_id = |kind| {
            builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id(kind)
                .unwrap()
        };
        let stone = builtin
            .get("minecraft:item")
            .unwrap()
            .id("minecraft:stone")
            .unwrap();
        let key = "minecraft:kebab";
        let painting = PaintingVariant {
            width: 1,
            height: 1,
            asset_id: key.into(),
            title: None,
            author: None,
        };
        let metadata = |id, values| C::SetEntityData(play::SetEntityData { id, values });
        let add = |id, kind, uuid, data| {
            C::AddEntity(play::AddEntity {
                id,
                kind: kind_id(kind),
                uuid,
                position: AT,
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data,
            })
        };
        let local = client.entity(7).unwrap();
        for (i, ((kind, _, synced, data), server_nbt)) in
            fixtures.iter().zip(&authoritative).enumerate()
        {
            let id = i as i32 + 100;
            let uuid = ints_uuid(entity_nbt::uuid(server_nbt).unwrap());
            client.handle_packet(add(id, kind, uuid, *data)).unwrap();
            let entity = EntityRef::new(local.epoch(), id as u32);
            if *kind == "minecraft:item" {
                assert_eq!(entity.client_nbt(client), None, "item without its stack");
            }
            let mut values = vec![
                (0, s::BYTE, Value::Byte(0x41)), // synced glow/fire are not saved flags
                (1, s::INT, Value::Int(145)),
                (2, s::OPTIONAL_COMPONENT, Value::OptText(Some(name.clone()))),
                (3, s::BOOLEAN, Value::Bool(true)),
                (4, s::BOOLEAN, Value::Bool(true)),
                (5, s::BOOLEAN, Value::Bool(true)),
                (7, s::INT, Value::Int(25)),
            ];
            values.extend(match *kind {
                "minecraft:armor_stand" => vec![(15, s::BYTE, Value::Byte(4))],
                "minecraft:item_frame" => vec![
                    (9, s::ITEM_STACK, Value::Item(Some(Stack::new(stone, 1)))),
                    (10, s::INT, Value::Int(3)),
                ],
                "minecraft:painting" => vec![(
                    9,
                    s::PAINTING_VARIANT,
                    Value::PaintingVariant(PaintingHolder::Reference {
                        id: 1,
                        name: key.into(),
                        variant: painting.clone(),
                    }),
                )],
                "minecraft:item" => {
                    vec![(8, s::ITEM_STACK, Value::Item(Some(Stack::new(stone, 1))))]
                }
                "minecraft:text_display" => vec![
                    (20, s::FLOAT, Value::Float(1.25)),
                    (
                        23,
                        s::COMPONENT,
                        Value::Text(Tag::String("Replica text".into())),
                    ),
                    (24, s::INT, Value::Int(84)),
                ],
                "minecraft:cow" => vec![
                    (9, s::FLOAT, Value::Float(7.0)),
                    (17, s::BOOLEAN, Value::Bool(true)),
                ],
                "minecraft:area_effect_cloud" => vec![(8, s::FLOAT, Value::Float(2.5))],
                _ => vec![],
            });
            client.handle_packet(metadata(id, values)).unwrap();
            let before = client.common.persist.uuids.len();
            let tag = Tag::Compound(
                entity
                    .client_nbt(client)
                    .unwrap_or_else(|| panic!("read {kind}")),
            );
            assert_eq!(client.common.persist.uuids.len(), before);
            for key in [
                "UUID",
                "Air",
                "CustomName",
                "CustomNameVisible",
                "Silent",
                "NoGravity",
                "TicksFrozen",
            ]
            .into_iter()
            .chain(synced.iter().copied())
            {
                assert_eq!(tag.get(key), server_nbt.get(key), "{kind}: {key}");
            }
            for key in ["id", "Glowing", "HasVisualFire"] {
                assert_eq!(tag.get(key), None, "{kind}: {key}");
            }
            assert_eq!(tag.get("Fire"), Some(&Tag::Short(0)), "{kind}");
            assert_eq!(tag.get("Invulnerable"), Some(&Tag::Byte(0)), "{kind}");
            assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(0)), "{kind}");
            match *kind {
                "minecraft:armor_stand" => assert_eq!(tag.get("DisabledSlots"), Some(&Tag::Int(0))),
                "minecraft:item_frame" => {
                    assert_eq!(tag.get("Fixed"), Some(&Tag::Byte(0)));
                    assert_eq!(
                        tag.get("ItemDropChance"),
                        Some(&Tag::Float(1.0_f32.to_bits()))
                    );
                }
                "minecraft:painting" => {
                    client
                        .handle_packet(metadata(
                            id,
                            vec![(
                                9,
                                s::PAINTING_VARIANT,
                                Value::PaintingVariant(PaintingHolder::Direct(painting.clone())),
                            )],
                        ))
                        .unwrap();
                    let inline = Tag::Compound(entity.client_nbt(client).unwrap());
                    assert_eq!(inline.get("variant"), None);
                    for key in ["facing", "block_pos", "UUID"] {
                        assert_eq!(inline.get(key), server_nbt.get(key));
                    }
                }
                "minecraft:item" => {
                    assert_eq!(tag.get("Age"), Some(&Tag::Short(0)));
                    assert_eq!(tag.get("PickupDelay"), Some(&Tag::Short(0)));
                }
                "minecraft:text_display" => assert_eq!(tag.get("start_interpolation"), None),
                "minecraft:cow" => {
                    assert_eq!(tag.get("Age"), Some(&Tag::Int(1)));
                    assert_eq!(tag.get("PersistenceRequired"), Some(&Tag::Byte(0)));
                }
                "minecraft:area_effect_cloud" => {
                    assert_eq!(tag.get("Duration"), Some(&Tag::Int(-1)));
                    assert_eq!(tag.get("WaitTime"), Some(&Tag::Int(20)));
                    assert_eq!(tag.get("potion_contents"), None);
                }
                _ => {}
            }
        }
        client
            .handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle: 105,
                passengers: vec![7, 106],
            }))
            .unwrap();
        let boat = Tag::Compound(client.entity(105).unwrap().client_nbt(client).unwrap());
        let Some(Tag::List(riders)) = boat.get("Passengers") else {
            panic!("saved boat has no riders")
        };
        assert_eq!(riders.len(), 1);
        assert_eq!(
            riders[0].get("id"),
            Some(&Tag::String("minecraft:cow".into()))
        );
        assert_eq!(riders[0].get("UUID"), authoritative[6].get("UUID"));

        client
            .handle_packet(add(120, "minecraft:ender_dragon", 120, 0))
            .unwrap();
        let part = sunrise_gameplay_common::game::dragon_common::dragon_part_id(120, 0);
        assert_eq!(EntityRef::new(local.epoch(), part).client_nbt(client), None);
        assert_eq!(local.client_nbt(client), None);
        assert_eq!(
            EntityRef::new(WorldEpoch::new(local.epoch().get() + 1), 100).client_nbt(client),
            None,
        );
        let removed = client.entity(100).unwrap();
        client
            .handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![100] }))
            .unwrap();
        assert_eq!(removed.client_nbt(client), None);
        assert!(
            sent(&mut reader).is_empty(),
            "replica NBT reads send no packets"
        );
    });
}
