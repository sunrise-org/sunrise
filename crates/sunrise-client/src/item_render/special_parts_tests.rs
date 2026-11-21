//! A registered renderer submits its baked geometry with the base model's display transforms,
//! particle and GUI light, in every display context and after a reload.
use super::*;
use mod_api::{
    __private::parts_installation::item_key, Component, ItemStack, SpecialModelBakeContext,
    SpecialModelCollector, SpecialModelFactory, SpecialModelGeometry, SpecialModelQuad,
    SpecialModelRenderer, SpecialModelStack, testing::FakeHost,
};
use std::fs;
use sunrise_assets::resource_source::{Resource, ResourceRoot};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PanConfig {
    handle: u8,
}

struct Pan {
    handle: u8,
    a: SpecialModelGeometry,
    b: SpecialModelGeometry,
    palette: Arc<[u32]>,
}

impl Pan {
    fn bake(config: PanConfig, bake: &dyn SpecialModelBakeContext) -> Result<Self, String> {
        Ok(Self {
            handle: config.handle,
            a: bake.bake_model("spt:item/a")?,
            b: bake.bake_model("spt:item/b")?,
            palette: Arc::new([0xFFCC_2211]),
        })
    }
}

struct PanTint;
impl Component for PanTint {
    type Value = i32;

    fn id(&self) -> &str {
        "spt:tint"
    }

    fn decode(&self, value: &Tag) -> Option<i32> {
        match value {
            Tag::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn encode(&self, value: i32) -> Tag {
        Tag::Int(value)
    }
}

const fn translated(x: f32) -> [f32; 12] {
    [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}
const DOUBLED: [f32; 12] = [2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0];

impl SpecialModelRenderer for Pan {
    type Argument = (u32, Option<i32>);

    fn extract_argument(
        &self,
        _: &dyn ClientHost,
        stack: &SpecialModelStack<'_>,
    ) -> Self::Argument {
        // Only the pan's component tints its geometry, not an unrelated stack's.
        let tint = if stack.item() == item_key(0) {
            stack.get(&PanTint)
        } else {
            None
        };
        (stack.count(), tint)
    }

    fn submit(
        &self,
        &(count, tint): &Self::Argument,
        context: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        let stack = |collector: &mut SpecialModelCollector<'_>, key, count| {
            collector.submit_stack(
                &ItemStack::new(item_key(key), count).unwrap(),
                ItemDisplayContext::Gui,
            )
        };
        let geometry = |collector: &mut SpecialModelCollector<'_>, model: &SpecialModelGeometry| {
            let tints = tint.map_or_else(
                || SpecialModelTints::Indexed(Arc::clone(&self.palette)),
                |tint| SpecialModelTints::Uniform(tint as u32),
            );
            collector.with_pose(translated(f32::from(self.handle) / 8.0), |pose| {
                pose.submit_geometry(model, tints)
            })
        };
        match count {
            1 => geometry(collector, &self.a),
            3 => {
                geometry(collector, &self.a)?;
                stack(collector, u32::MAX, 1)
            }
            4 => {
                geometry(collector, &self.a)?;
                Err("the test renderer refuses this stack".into())
            }
            5 => collector.with_pose(DOUBLED, |pose| {
                pose.submit_geometry(&self.a, SpecialModelTints::Untinted)
            }),
            6 | 7 => collector.submit_geometry(
                &self.a,
                SpecialModelTints::Uniform([WHITE, GREY][count as usize - 6] as u32),
            ),
            // A tinted, foiled cube beside B; then the pan itself, a cycle.
            8 => {
                collector.with_pose(translated(0.25), |pose| stack(pose, 2, 1))?;
                geometry(collector, &self.b)
            }
            9 => collector.with_pose(translated(0.25), |pose| stack(pose, 0, 9)),
            // A to B to A with geometry staged at both levels.
            10 => {
                geometry(collector, &self.b)?;
                stack(collector, 0, 11)
            }
            11 => {
                geometry(collector, &self.a)?;
                stack(collector, 0, 10)
            }
            // Ignore the nested refusal on purpose: neither layer may commit.
            12 | 13 => {
                let (model, inner, count) = if count == 12 {
                    (&self.b, 0, 13)
                } else {
                    (&self.a, 3, 12)
                };
                geometry(collector, model)?;
                let _ = stack(collector, inner, count);
                let _ = geometry(collector, model);
                Ok(())
            }
            14 => {
                geometry(collector, &self.b)?;
                stack(collector, 4, 4)
            }
            15 => {
                geometry(collector, &self.a)?;
                panic!("the pan fell")
            }
            16 => {
                geometry(collector, &self.a)?;
                // Both input transforms are finite; their composed pose is not.
                let _ = collector.with_pose(DOUBLED, |pose| {
                    pose.with_pose(translated(f32::MAX), |_| Ok(()))
                });
                Ok(())
            }
            20 | 21 | 24 => {
                let cubes = if count == 24 { 63 } else { 44 + count };
                for _ in 0..cubes {
                    stack(collector, 1, 1)?;
                }
                Ok(())
            }
            22 | 23 => {
                geometry(collector, &self.b)?;
                stack(collector, 0, if count == 22 { 20 } else { 24 })
            }
            25 => {
                collector.with_pose(translated(0.25), |pose| {
                    pose.with_pose(DOUBLED, |pose| {
                        pose.submit_geometry(&self.a, SpecialModelTints::Untinted)
                    })?;
                    pose.submit_geometry(&self.a, SpecialModelTints::Untinted)
                })?;
                collector.submit_geometry(&self.a, SpecialModelTints::Untinted)
            }
            30 => {
                let lease = if context.display == ItemDisplayContext::Ground
                    && context.game_time == Some(41)
                {
                    &self.b
                } else {
                    &self.a
                };
                let tints = tint.map_or(SpecialModelTints::Untinted, |tint| {
                    SpecialModelTints::Uniform(tint as u32)
                });
                collector.submit_geometry(lease, tints)
            }
            _ => geometry(collector, &self.b),
        }
    }
}

/// Bridge the fixture's synthetic items to the same collector as the game uses.
struct PanEnv(FakeHost);
impl ItemEnv for PanEnv {}
impl ItemDrawEnv for PanEnv {
    fn special_model_input(
        &self,
        _: &BakedSpecialModel,
        stack: &StackView,
        context: &Context<'_>,
    ) -> Option<SpecialModelInput<'_>> {
        let item = match stack.item.as_str() {
            "spt:pan" => 0,
            "spt:composite" => 3,
            "spt:fault" | "spt:alias" => 4,
            "spt:bundle" => 5,
            _ => return None,
        };
        let display = ItemDisplayContext::ALL
            .into_iter()
            .find(|value| value.name() == context.display.name())?;
        Some(SpecialModelInput {
            host: &self.0,
            item: item_key(item),
            count: NonZeroU32::new(stack.count.try_into().ok()?)?,
            context: SpecialModelContext::new(
                display,
                context.level.map(|level| level.game_time),
                0.25,
            ),
        })
    }

    fn special_model_stack(
        &self,
        inner: &ItemStack,
        display: ItemDisplayContext,
    ) -> Option<(StackView, DisplayContext)> {
        assert_eq!(display, ItemDisplayContext::Gui);
        let (item, glint) = match inner.item() {
            key if key == item_key(0) => ("spt:pan", false),
            key if key == item_key(1) => ("spt:cube", false),
            key if key == item_key(2) => ("spt:grey_cube", true),
            key if key == item_key(3) => ("spt:composite", false),
            key if key == item_key(4) => ("spt:fault", false),
            _ => return None,
        };
        let mut stack = stack(item, inner.count() as i32);
        if glint {
            stack.patch.insert(
                "minecraft:enchantment_glint_override".into(),
                Some(serde_json::json!(true)),
            );
        }
        Some((stack, DisplayContext::Gui))
    }
}

fn pan_factories(on_bake: impl Fn(u8) + Send + Sync + 'static) -> SpecialModelFactories {
    BTreeMap::from([(
        "spt:pan".into(),
        SpecialModelFactory::new(
            |config| config.decode::<PanConfig>(),
            move |config, bake| {
                on_bake(config.handle);
                Pan::bake(config, bake)
            },
        ),
    )])
}

/// White and an encoded 0.5 grey, packed vanilla-space ARGB.
const WHITE: i32 = 0xFFFF_FFFF_u32 as i32;
const GREY: i32 = 0xFF80_8080_u32 as i32;

/// `n` separated 4-pixel cubes, every face tinted by index 0.
fn cubes(n: usize) -> String {
    let faces = ["north", "east", "south", "west", "up", "down"]
        .map(|f| format!(r##""{f}":{{"uv":[0,0,4,4],"texture":"#all","tintindex":0}}"##))
        .join(",");
    let elements = (0..n)
        .map(|i| {
            let x = i * 5;
            format!(
                r#"{{"from":[{x},0,0],"to":[{},4,4],"faces":{{{faces}}}}}"#,
                x + 4
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"textures":{{"all":"spt:item/grey"}},"elements":[{elements}]}}"#)
}

fn write(root: &std::path::Path, name: &str, data: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// A pan node between ordinary grey and white cube models, to test layer-local refusal.
fn write_composite(root: &std::path::Path) {
    write(
        root,
        "assets/spt/items/composite.json",
        &format!(
            r#"{{"model":{{"type":"composite","models":[
            {{"type":"model","model":"spt:item/a","tints":[{{"type":"constant","value":{GREY}}}]}},
            {{"type":"special","base":"spt:item/base","model":{{"type":"spt:pan","handle":2}}}},
            {{"type":"model","model":"spt:item/b","tints":[{{"type":"constant","value":{WHITE}}}]}}
        ]}}}}"#
        ),
    );
}

/// The packs in `root`, with B of `b_cubes` cubes.
fn renderer(
    root: &std::path::Path,
    b_cubes: usize,
    handle: u8,
    factories: &SpecialModelFactories,
) -> ItemRenderer {
    write(root, "items.json", "{}");
    write(
        root,
        "assets/minecraft/atlases/items.json",
        r#"{"sources":[{"type":"single","resource":"spt:item/grey"},{"type":"single","resource":"spt:item/flour"}]}"#,
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/grey.png"),
        16,
        16,
        [128, 128, 128, 255],
    );
    super::glint_tests::png(
        &root.join("assets/spt/textures/item/flour.png"),
        16,
        16,
        [250, 250, 250, 255],
    );
    write(
        root,
        "assets/spt/items/pan.json",
        &format!(
            r#"{{"model":{{"type":"minecraft:special","base":"spt:item/base","model":{{"type":"spt:pan","handle":{handle}}}}}}}"#
        ),
    );
    write(
        root,
        "assets/spt/items/cube.json",
        r#"{"model":{"type":"minecraft:model","model":"spt:item/a"}}"#,
    );
    // Cube A tinted white or grey by vanilla's constant tint source.
    for (name, tint) in [("white_cube", WHITE), ("grey_cube", GREY)] {
        write(
            root,
            &format!("assets/spt/items/{name}.json"),
            &format!(
                r#"{{"model":{{"type":"minecraft:model","model":"spt:item/a","tints":[{{"type":"minecraft:constant","value":{tint}}}]}}}}"#
            ),
        );
    }
    write(
        root,
        "assets/spt/items/stranger.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:unregistered"}}}"#,
    );
    // The base has no geometry of its own: its display transforms differ by context, its
    // particle is flour and its GUI light is flat.
    write(
        root,
        "assets/spt/models/item/base.json",
        r#"{"gui_light":"front","textures":{"particle":"spt:item/flour"},"display":{
            "ground":{"scale":[0.5,0.5,0.5]},
            "fixed":{"rotation":[0,180,0]},
            "head":{"translation":[0,13,7]},
            "thirdperson_righthand":{"rotation":[75,45,0],"translation":[0,2.5,0]},
            "gui":{"rotation":[30,225,0],"scale":[0.625,0.625,0.625]}}}"#,
    );
    write(root, "assets/spt/models/item/a.json", &cubes(1));
    write(root, "assets/spt/models/item/b.json", &cubes(b_cubes));
    let data = crate::test_resources::item_data(
        root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let root = ResourceRoot::directory(root.to_path_buf());
    ItemRenderer::load_tables(
        &root,
        crate::model_library::ParsedModels::load(root.source()).into_item_library(),
        true,
        &data,
        None,
        Some(factories),
    )
    .unwrap()
}

fn stack(item: &str, count: i32) -> StackView {
    let mut stack = StackView::new(item, count);
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(serde_json::json!(item)));
    stack
}

#[test]
fn each_special_draw_reads_effective_components_and_current_context() {
    let root = tempfile::tempdir().unwrap();
    let mut renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    Arc::make_mut(&mut renderer.models.prototypes).insert(
        "spt:pan".into(),
        BTreeMap::from([("spt:tint".into(), serde_json::json!(GREY))]),
    );
    let env = PanEnv(FakeHost::new());
    let mut pan = stack("spt:pan", 30);
    let level = |game_time| LevelState {
        game_time,
        ..Default::default()
    };
    let draw = |pan: &StackView, display, time| {
        let state = renderer.resolve(pan, display, None, Some(&level(time)), 0, &env);
        quads(&renderer, &state)
            .into_iter()
            .map(|quad| quad.color)
            .collect::<Vec<_>>()
    };

    // Same stack, same view, and no intervening frame: both time and display matter.
    assert_eq!(
        draw(&pan, DisplayContext::Ground, 41),
        vec![GREY as u32; 12]
    );
    assert_eq!(draw(&pan, DisplayContext::Ground, 42), vec![GREY as u32; 6]);
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![GREY as u32; 6]);
    pan.patch.insert(
        "spt:tint".into(),
        Some(serde_json::json!(0xFFAA_1100_u32 as i32)),
    );
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![0xFFAA_1100; 6]);
    pan.patch.insert("spt:tint".into(), None);
    assert_eq!(draw(&pan, DisplayContext::Gui, 41), vec![u32::MAX; 6]);
}

#[test]
fn a_fault_rolls_back_and_disables_the_type_only_in_its_resource_generation() {
    let root = tempfile::tempdir().unwrap();
    write_composite(root.path());
    write(
        root.path(),
        "assets/spt/items/alias.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:pan","handle":1}}}"#,
    );
    let factories = pan_factories(|_| {});
    let env = PanEnv(FakeHost::new());
    let draw = |renderer: &ItemRenderer, stack: &StackView| {
        renderer.resolve(stack, DisplayContext::None, None, None, 0, &env)
    };
    let is_missing_layer = |renderer: &ItemRenderer, layer: &LayerRender| {
        matches!(&layer.geometry, LayerGeometry::Quads(model)
            if Arc::ptr_eq(model, &renderer.bake(MISSING_MODEL)))
    };
    let is_missing = |renderer: &ItemRenderer, stack: &StackView| {
        let state = draw(renderer, stack);
        matches!(state.layers.as_slice(), [layer] if is_missing_layer(renderer, layer))
    };
    let working = stack("spt:alias", 1);
    let mut unreadable = stack("spt:unreadable", 1);
    unreadable.patch.insert(
        "minecraft:item_model".into(),
        Some(serde_json::json!("spt:pan")),
    );
    for count in [4, 15, 16] {
        let generation = renderer(root.path(), 2, 2, &factories);
        assert!(is_missing(&generation, &unreadable));
        assert_eq!(quads(&generation, &draw(&generation, &working)).len(), 6);
        // Err after a write, panic after a write, and an ignored non-finite composed pose.
        let state = draw(&generation, &stack("spt:composite", count));
        let [_, replacement, _] = state.layers.as_slice() else {
            panic!("count {count}: the failed layer leaked geometry or erased a sibling");
        };
        assert!(is_missing_layer(&generation, replacement));
        // The fixture atlas omits the missing sprite: only the two ordinary siblings draw.
        let colors: Vec<_> = quads(&generation, &state)
            .into_iter()
            .map(|quad| quad.color)
            .collect();
        assert_eq!(colors.len(), 18);
        assert_eq!(colors[..6], [GREY as u32; 6]);
        assert_eq!(colors[6..], [WHITE as u32; 12]);
        assert!(
            is_missing(&generation, &working),
            "the sibling node of the same type was not disabled"
        );
    }
    let reloaded = renderer(root.path(), 2, 2, &factories);
    assert_eq!(quads(&reloaded, &draw(&reloaded, &working)).len(), 6);
}

fn sprites() -> HashMap<String, Rect> {
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    HashMap::from([
        ("spt:item/grey".to_owned(), rect),
        ("spt:item/flour".to_owned(), rect),
    ])
}

fn quads(renderer: &ItemRenderer, state: &ItemRenderState) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    state.submit(renderer, &sprites(), &NoHost, Mat4::IDENTITY, &mut quads);
    quads
}

#[test]
fn an_invalid_winning_registered_model_does_not_load_a_lower_pack() {
    let item_path = "assets/spt/items/pan.json".to_owned();
    let lower = Resource::bytes(
        br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:pan","handle":2}}}"#.to_vec(),
    );
    let winner = Resource::bytes(
        br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:pan","handle":null}}}"#.to_vec(),
    );
    let files = BTreeMap::from([
        (item_path.clone(), winner.clone()),
        ("assets/spt/models/item/a.json".to_owned(), Resource::bytes(b"{}".to_vec())),
        ("assets/spt/models/item/b.json".to_owned(), Resource::bytes(b"{}".to_vec())),
        ("assets/spt/items/other.json".to_owned(), lower.clone()),
        ("assets/spt/items/decode-panic.json".to_owned(), Resource::bytes(
            br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:decode-panic"}}}"#.to_vec(),
        )),
        ("assets/spt/items/bake-panic.json".to_owned(), Resource::bytes(
            br#"{"model":{"type":"special","base":"spt:item/base","model":{"type":"spt:bake-panic"}}}"#.to_vec(),
        )),
    ]);
    let pack = ResourceSource::indexed_with_layers(
        files,
        BTreeMap::from([(item_path, vec![lower, winner])]),
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path();
    write(directory, "items.json", "{}");
    let item_data = crate::test_resources::item_data(
        directory,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let root = ResourceRoot::new(directory.to_owned(), directory.to_owned(), pack);
    let mut factories = pan_factories(|_| {});
    factories.extend([
        (
            "spt:decode-panic".into(),
            SpecialModelFactory::new(
                |_| -> Result<Pan, String> { panic!("decoder panicked") },
                |pan, _| Ok(pan),
            ),
        ),
        (
            "spt:bake-panic".into(),
            SpecialModelFactory::new(
                |_| Ok(()),
                |(), _| -> Result<Pan, String> { panic!("baker panicked") },
            ),
        ),
    ]);
    let renderer = ItemRenderer::load_tables(
        &root,
        crate::model_library::ParsedModels::load(root.source()).into_item_library(),
        true,
        &item_data,
        None,
        Some(&factories),
    )
    .unwrap();
    let definitions = &renderer.models.definitions;
    assert_eq!(
        definitions.keys().map(String::as_str).collect::<Vec<_>>(),
        ["spt:other"]
    );
    assert!(matches!(
        &definitions["spt:other"].model,
        sunrise_assets::item_model::ItemModelNode::RegisteredSpecial { .. },
    ));
}

#[test]
fn a_generation_bakes_typed_config_once_and_draws_use_the_baked_instance() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let factories = pan_factories({
        let calls = Arc::clone(&calls);
        move |handle| calls.lock().unwrap().push(handle)
    });
    let draw = |renderer: &ItemRenderer| {
        let env = PanEnv(FakeHost::new());
        let state = renderer.resolve(
            &stack("spt:pan", 1),
            DisplayContext::None,
            None,
            None,
            0,
            &env,
        );
        assert_eq!(quads(renderer, &state).len(), 6);
        let (lo, hi) = state.bounding_box();
        (lo.x, hi.x)
    };
    let first = renderer(temp.path(), 2, 2, &factories);
    assert_eq!(*calls.lock().unwrap(), [2]);
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(*calls.lock().unwrap(), [2]);

    let second = renderer(temp.path(), 2, 4, &factories);
    assert_eq!(*calls.lock().unwrap(), [2, 4]);
    assert_eq!(draw(&second), (0.0, 0.25));
    assert_eq!(draw(&first), (-0.25, 0.0));
    assert_eq!(draw(&second), (0.0, 0.25));
    assert_eq!(*calls.lock().unwrap(), [2, 4]);
}

#[test]
fn special_geometry_leases_resolve_pack_sprites_and_reject_invalid_quads() {
    let temp = tempfile::tempdir().unwrap();
    let leases = Arc::new(Mutex::new(None));
    let bake = {
        let leases = Arc::clone(&leases);
        move |pan, context: &dyn mod_api::SpecialModelBakeContext| {
            let positions = [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ];
            let mut quad = SpecialModelQuad::new(
                positions,
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                "spt:item/grey",
            );
            quad.tint_index = Some(2);
            quad.light_emission = 12;
            quad.force_translucent = true;
            let resource = context.bake_model("spt:item/b")?;
            let custom = context.bake_geometry(&[quad.clone()])?;
            let invalid: [(&str, fn(&mut SpecialModelQuad)); 6] = [
                ("non-finite", |q| q.positions[0][0] = f32::NAN),
                ("facing", |q| q.positions = [[0.0; 3]; 4]),
                ("UV", |q| q.uv[0][0] = -1.0),
                ("light", |q| q.light_emission = 16),
                ("tint", |q| q.tint_index = Some(u32::MAX)),
                ("missing", |q| q.sprite = "spt:item/not-present".into()),
            ];
            for (message, invalidate) in invalid {
                let mut invalid_quad = quad.clone();
                invalidate(&mut invalid_quad);
                match context.bake_geometry(&[invalid_quad]) {
                    Err(error) if error.contains(message) => {}
                    _ => return Err(format!("did not reject invalid {message}")),
                }
            }
            *leases.lock().unwrap() = Some((resource, custom.clone()));
            Ok(Pan {
                b: custom,
                ..Pan::bake(pan, context)?
            })
        }
    };
    let factories = BTreeMap::from([(
        "spt:pan".into(),
        SpecialModelFactory::new(|config| config.decode::<PanConfig>(), bake),
    )]);
    let renderer = renderer(temp.path(), 2, 2, &factories);
    assert!(renderer.models.definitions.contains_key("spt:pan"));
    let (resource, custom) = leases.lock().unwrap().take().unwrap();
    let resource = resource.downcast_ref::<BakedSpecialGeometry>().unwrap();
    assert_eq!(resource.model.quads.len(), 12);
    let custom = custom.downcast_ref::<BakedSpecialGeometry>().unwrap();
    assert_eq!(
        custom.extents.as_slice(),
        [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y]
    );
    let [quad] = custom.model.quads.as_slice() else {
        panic!("expected one custom quad")
    };
    assert_eq!(quad.facing, Direction::South);
    assert_eq!(quad.material.sprite, "spt:item/grey");
    assert!(quad.material.force_translucent);
    assert_eq!(quad.tint_index, 2);
    assert_eq!(quad.light_emission, 12);

    let mut stack = stack("spt:pan", 2);
    stack.patch.insert(
        "minecraft:enchantment_glint_override".into(),
        Some(serde_json::json!(true)),
    );
    let state = renderer.resolve(
        &stack,
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    let quads = quads(&renderer, &state);
    let [quad] = quads.as_slice() else {
        panic!("expected the submitted custom quad");
    };
    assert_eq!(quad.light_emission, 12);
    assert_eq!(
        quad.color,
        u32::MAX,
        "the indexed palette has no entry for tint 2"
    );
    assert_eq!(quad.foil, Foil::Standard);
    assert!(quad.translucent);
}

#[test]
fn child_poses_leave_following_submissions_in_their_own_scopes() {
    let root = tempfile::tempdir().unwrap();
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let state = renderer.resolve(
        &stack("spt:pan", 25),
        DisplayContext::None,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    let quads = quads(&renderer, &state);
    assert_eq!(quads.len(), 18);
    // The 1/4-wide cube, centered by -1/2 after the local pose: translated by 1/4 and
    // doubled in the child, only translated in its parent, then unposed at the root.
    for (cube, expected) in
        quads
            .as_chunks::<6>()
            .0
            .iter()
            .zip([(-0.25, 0.25), (-0.25, 0.0), (-0.5, -0.25)])
    {
        let bounds = cube
            .iter()
            .flat_map(|quad| quad.pos)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), pos| {
                (lo.min(pos.x), hi.max(pos.x))
            });
        assert_eq!(bounds, expected);
    }
}

#[test]
fn a_bundle_extracts_the_selected_stacks_argument_and_foil() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "assets/spt/items/bundle.json",
        r#"{"model":{"type":"bundle/selected_item"}}"#,
    );
    let renderer = renderer(root.path(), 2, 2, &pan_factories(|_| {}));
    let mut bundle = stack("spt:bundle", 1);
    bundle.bundle_selected = 1;
    bundle
        .patch
        .insert("spt:tint".into(), Some(serde_json::json!(GREY)));
    bundle.patch.insert(
        "minecraft:bundle_contents".into(),
        Some(serde_json::json!([
            {"id": "spt:pan", "count": 1, "components": {"minecraft:item_model": "spt:pan"}},
            {"id": "spt:pan", "count": 2, "components": {
                "minecraft:item_model": "spt:pan",
                "minecraft:enchantment_glint_override": true,
                "spt:tint": 0xFFAA_1100_u32 as i32
            }}
        ])),
    );
    let state = renderer.resolve(
        &bundle,
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    // Two pans draw B, not A. The tint only comes from a pan's own component and foil only
    // from the selected stack's override; the outer bundle and first entry are not foiled.
    let drawn: Vec<_> = quads(&renderer, &state)
        .into_iter()
        .map(|quad| (quad.color, quad.foil))
        .collect();
    assert_eq!(drawn, [(0xFFAA_1100, Foil::Standard); 12]);
}

#[test]
#[ignore = "daily"]
fn a_registered_special_model_draws_its_renderers_resource_models() {
    let root = std::env::temp_dir().join(format!("sunrise-special-parts-{}", std::process::id()));
    write_composite(&root);
    write(
        &root,
        "assets/spt/items/fault.json",
        r#"{"model":{"type":"minecraft:special","base":"spt:item/base","model":{"type":"spt:fault","handle":2}}}"#,
    );
    let mut factories = pan_factories(|_| {});
    factories.insert("spt:fault".into(), factories["spt:pan"].clone());
    let renderer = renderer(&root, 2, 2, &factories);
    let env = PanEnv(FakeHost::new());
    let resolve = |item: &str, count, display| {
        renderer.resolve(&stack(item, count), display, None, None, 0, &env)
    };

    // A or B, in the inventory and in hand, tinted and with the base's particle and GUI light.
    for display in [DisplayContext::Gui, DisplayContext::FirstPersonRightHand] {
        let one = resolve("spt:pan", 1, display);
        let two = resolve("spt:pan", 2, display);
        assert_eq!(quads(&renderer, &one).len(), 6, "{display:?}");
        assert_eq!(quads(&renderer, &two).len(), 12, "{display:?}");
        assert!(
            quads(&renderer, &two)
                .iter()
                .all(|q| q.color == 0xFFCC_2211)
        );
        assert_eq!(two.layers[0].particle.as_ref(), "spt:item/flour");
        assert!(Arc::ptr_eq(
            &one.layers[0].particle,
            &two.layers[0].particle
        ));
        assert!(!two.uses_block_light());
        assert!(
            two.animated,
            "a mod's drawing is never kept as a static GUI item"
        );
    }

    // B on the ground, in a frame, on a head and in another player's hand: the base's
    // transform for each context, so each draws B somewhere else.
    let contexts = [
        DisplayContext::Ground,
        DisplayContext::Fixed,
        DisplayContext::Head,
        DisplayContext::ThirdPersonRightHand,
    ];
    let matrices: Vec<Mat4> = contexts
        .iter()
        .map(|&display| {
            let state = resolve("spt:pan", 2, display);
            assert_eq!(quads(&renderer, &state).len(), 12, "{display:?}");
            state.layers[0].matrix
        })
        .collect();
    for (i, a) in matrices.iter().enumerate() {
        for b in &matrices[i + 1..] {
            assert_ne!(a, b);
        }
    }
    // A part's pose is about the model's corner, as its elements are: A's cube spans x 0 to 1/4
    // there, moved a quarter block or doubled; the drawn item then sits half a block back.
    let x = |count| {
        let (lo, hi) = resolve("spt:pan", count, DisplayContext::None).bounding_box();
        (lo.x, hi.x)
    };
    assert_eq!(x(1), (-0.25, 0.0));
    assert_eq!(x(5), (-0.5, 0.0));

    // An unreadable nested stack discards the geometry already submitted, without poisoning
    // the parent. The next valid stack of that type still draws.
    let missing = renderer.bake(MISSING_MODEL);
    let is_missing_layer = |layer: &LayerRender| matches!(&layer.geometry, LayerGeometry::Quads(baked) if Arc::ptr_eq(baked, &missing));
    let is_missing = |state: &ItemRenderState| matches!(state.layers.as_slice(), [layer] if is_missing_layer(layer));
    assert!(is_missing(&resolve("spt:pan", 3, DisplayContext::Gui)));
    // A stack part draws the inner item through the resolver, under its own display context and
    // tints and foil, not the part's: the cube's six glinting grey quads beside B's twelve
    // tinted ones. A stack part that names the stack being drawn is a cycle, refused whole.
    let nested = resolve("spt:pan", 8, DisplayContext::Fixed);
    let draws: Vec<_> = quads(&renderer, &nested)
        .iter()
        .map(|q| (q.color, q.foil))
        .collect();
    assert_eq!(nested.layers[0].display, DisplayContext::Gui);
    assert_eq!(nested.layers[1].display, DisplayContext::Fixed);
    assert_eq!(draws[..6], [(GREY as u32, Foil::Standard); 6]);
    assert_eq!(draws[6..], [(0xFFCC_2211, Foil::None); 12]);
    assert!(nested.animated);
    assert!(is_missing(&resolve("spt:pan", 9, DisplayContext::Gui)));
    // A refusal below a first level refuses the answers above it whole, their valid sibling
    // models too; the expansion budget is 64 for the whole draw, below the first level as well.
    assert!(is_missing(&resolve("spt:pan", 10, DisplayContext::Gui)));
    assert!(is_missing(&resolve("spt:pan", 11, DisplayContext::Gui)));
    let composite = resolve("spt:composite", 12, DisplayContext::Gui);
    let [before, replacement, after] = composite.layers.as_slice() else {
        panic!("the failed parent and child must leave just one replacement between siblings");
    };
    assert!(is_missing_layer(replacement));
    assert!(matches!(&before.geometry, LayerGeometry::Quads(baked) if baked.quads.len() == 6));
    assert!(matches!(&after.geometry, LayerGeometry::Quads(baked) if baked.quads.len() == 12));
    assert_eq!(before.tints.at(0), GREY as u32);
    assert_eq!(after.tints.at(0), WHITE as u32);
    assert_eq!(
        quads(&renderer, &resolve("spt:pan", 1, DisplayContext::Gui)).len(),
        6
    );
    assert!(is_missing(&resolve("spt:pan", 14, DisplayContext::Gui)));
    // The nested callback's own type is disabled, not the parent that submitted it.
    assert!(is_missing(&resolve("spt:fault", 1, DisplayContext::Gui)));
    let quads_of = |count| quads(&renderer, &resolve("spt:pan", count, DisplayContext::Gui)).len();
    assert_eq!(quads_of(20), 64 * 6);
    assert!(is_missing(&resolve("spt:pan", 21, DisplayContext::Gui)));
    assert!(is_missing(&resolve("spt:pan", 22, DisplayContext::Gui)));
    assert_eq!(quads_of(23), 12 + 63 * 6);
    assert!(!is_missing(&resolve("spt:pan", 1, DisplayContext::Gui)));
    assert_eq!(
        quads(&renderer, &resolve("spt:cube", 1, DisplayContext::Gui)).len(),
        6
    );

    // A special type no mod registered isn't loaded: its item draws the missing model.
    assert!(is_missing(&resolve("spt:stranger", 1, DisplayContext::Gui)));

    // A reload that makes B three cubes draws 18 quads for the same stack.
    let reloaded = super::special_parts_tests::renderer(&root, 3, 2, &factories);
    let state = reloaded.resolve(
        &stack("spt:pan", 2),
        DisplayContext::Gui,
        None,
        None,
        0,
        &PanEnv(FakeHost::new()),
    );
    assert_eq!(quads(&reloaded, &state).len(), 18);
    fs::remove_dir_all(root).unwrap();
}

/// Row 5 of the matrix (`docs/design/mod-api/resource-renderers.md`), an old state across a
/// reload. A part's quads are CPU vertices submitted every frame, not prepared GPU models, so the
/// state's own `Arc<BakedItemModel>` is the only lease and there is no retirement step: a state
/// resolved before a reload still draws its 12 quads after it, the old baked model is released
/// exactly once, when the last state holding it drops, and a new resolve draws the reload's 18.
#[test]
#[ignore = "daily"]
fn an_old_state_draws_across_a_reload_and_releases_its_models_once() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-special-parts-lease-{}",
        std::process::id()
    ));
    let factories = pan_factories(|_| {});
    let before = renderer(&root, 2, 2, &factories);
    let resolve = |renderer: &ItemRenderer| {
        renderer.resolve(
            &stack("spt:pan", 2),
            DisplayContext::Gui,
            None,
            None,
            0,
            &PanEnv(FakeHost::new()),
        )
    };
    let (old, also_old) = (resolve(&before), resolve(&before));
    let LayerGeometry::Quads(b) = &old.layers[0].geometry else {
        panic!("not quads: {:?}", old.layers[0].geometry);
    };
    let b = Arc::downgrade(b);
    let after = renderer(&root, 3, 2, &factories);
    drop(before);
    assert_eq!(quads(&after, &old).len(), 12);
    assert_eq!(quads(&after, &resolve(&after)).len(), 18);
    assert_eq!(b.strong_count(), 2, "the two old states hold the old B");
    drop(old);
    assert_eq!(b.strong_count(), 1);
    assert_eq!(quads(&after, &also_old).len(), 12);
    drop(also_old);
    assert_eq!(b.strong_count(), 0, "the old B outlived its last state");
    fs::remove_dir_all(root).unwrap();
}

/// Row 11 of the matrix, a part's tint. It is packed vanilla-space ARGB, as an item model's
/// tints are: white and an encoded 0.5 reach the vertices as the colour vanilla's constant tint
/// source gives the same cube, not decoded, so the frame decodes them once (vanilla's power 2.2:
/// shader values 1 and about 0.21763764, pinned by `gfx::linear_blend_tests`). A value above 1 has
/// no input.
#[test]
#[ignore = "daily"]
fn a_parts_tint_reaches_the_vertices_as_a_vanilla_tint_does() {
    let root =
        std::env::temp_dir().join(format!("sunrise-special-parts-tint-{}", std::process::id()));
    let factories = pan_factories(|_| {});
    let renderer = renderer(&root, 2, 2, &factories);
    let env = PanEnv(FakeHost::new());
    let colours = |item: &str, count| {
        let state = renderer.resolve(
            &stack(item, count),
            DisplayContext::Gui,
            None,
            None,
            0,
            &env,
        );
        quads(&renderer, &state)
            .iter()
            .map(|q| q.color)
            .collect::<Vec<_>>()
    };
    for (count, vanilla, argb) in [(6, "spt:white_cube", WHITE), (7, "spt:grey_cube", GREY)] {
        let part = colours("spt:pan", count);
        assert_eq!(part, colours(vanilla, 1), "{vanilla}");
        assert_eq!(part, vec![argb as u32; 6], "{vanilla}");
    }
    fs::remove_dir_all(root).unwrap();
}
