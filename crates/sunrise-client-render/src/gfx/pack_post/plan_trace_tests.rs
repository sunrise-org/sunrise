//! The frame trace (`plan-trace` builds only): frames drawn through `PackPost::run` are written as the runtime
//! resolved them, and the frame planner's checks hold on them.
use super::destination_tests::{Coverage, Fixture, pack};
use super::*;
use crate::gfx::Frame;
use crate::hud::HudBatch;
use std::io::{Cursor, Write};
use sunrise_frame_plan::measure::{Callbacks, measure};
use sunrise_frame_plan::record::{Recorded, Start};
use sunrise_frame_plan::trace::{Load, Op, Use, check_blocks, project};
use sunrise_render_shaderpack::pack_post::plan_trace::{
    CALLBACK_DREW, DEFERRED_SKIPPED_THEN_RAN, Recorder,
};

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main(){uv=gl_MultiTexCoord0.xy;gl_Position=ftransform();}\n";

/// composite writes colortex1 (cleared every frame); composite1 reads it and writes colortex2, which `final` shows;
/// composite2 writes colortex3, which nothing reads.
fn post() -> sunrise_shaders::Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    for (name, body) in [
        (
            "composite",
            "/* RENDERTARGETS:1 */\nvoid main(){gl_FragData[0]=vec4(0.25);}",
        ),
        (
            "composite1",
            "/* RENDERTARGETS:2 */\nuniform sampler2D colortex1;varying vec2 uv;\nvoid main(){gl_FragData[0]=texture2D(colortex1,uv);}",
        ),
        (
            "composite2",
            "/* RENDERTARGETS:3 */\nvoid main(){gl_FragData[0]=vec4(0.5);}",
        ),
        (
            "final",
            "uniform sampler2D colortex2;varying vec2 uv;\nvoid main(){gl_FragColor=texture2D(colortex2,uv);}",
        ),
    ] {
        add(&format!("{name}.vsh"), VERTEX);
        add(&format!("{name}.fsh"), &format!("#version 120\n{body}\n"));
    }
    let pack =
        sunrise_shaders::Pack::from_zip_bytes("plan-trace", zip.finish().unwrap().into_inner())
            .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

fn frames(dir: &std::path::Path) -> Vec<Recorded> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| Recorded::parse(&std::fs::read_to_string(f).unwrap()).expect("a recorded frame"))
        .collect()
}

/// The label of `work` in `frame`.
fn label(frame: &Recorded, work: u32) -> &str {
    &frame.works.iter().find(|(w, _)| *w == work).unwrap().1
}

/// Each unit of work outside blocks, by label, with its uses as (texture label, use).
fn units(frame: &Recorded) -> Vec<(String, Vec<(String, Use)>)> {
    let texture = |id| {
        frame
            .textures
            .iter()
            .find(|n| n.id == id)
            .map_or("?".to_owned(), |n| n.label.clone())
    };
    let mut depth = 0;
    let mut out = Vec::new();
    for op in &frame.frame.ops {
        match op {
            Op::BlockStart(_) => depth += 1,
            Op::BlockEnd(_) => depth -= 1,
            Op::Run { work, touches, .. } if depth == 0 => out.push((
                label(frame, *work).to_owned(),
                touches
                    .iter()
                    .map(|t| (texture(t.texture), t.how))
                    .collect(),
            )),
            Op::Run { .. } => {}
        }
    }
    out
}

/// A recorder writing its first `frames` frames to a fresh folder of this test's own.
fn tracing(post: &mut PackPost, name: &str, frames: u32) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("plan-trace-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    post.plan_trace = Some(Box::new(std::sync::Mutex::new(Recorder::new(
        dir.clone(),
        frames,
    ))));
    dir
}

/// The frames written to `dir`, which is removed, each checked: no error, a projection, every block's touches
/// inside its declared set.
fn checked(dir: &std::path::Path) -> Vec<Recorded> {
    let recorded = frames(dir);
    let _ = std::fs::remove_dir_all(dir);
    for frame in &recorded {
        assert_eq!(frame.errors, Vec::<String>::new(), "frame {}", frame.seq);
        project(&frame.frame).expect("projects");
        let declared: Vec<_> = frame
            .blocks
            .iter()
            .map(|b| (b.block, b.declared.clone()))
            .collect();
        assert_eq!(
            check_blocks(&frame.frame, &declared),
            Ok(()),
            "frame {}",
            frame.seq
        );
    }
    recorded
}

fn guard(frame: &Recorded, name: &str) -> Option<bool> {
    frame
        .guards
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| *v)
}

#[test]
fn drawn_frames_are_recorded_as_resolved_and_pass_the_planner_s_checks() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(post());
    let dir = tracing(&mut post, "drawn", 3);
    for _ in 0..4 {
        gpu.render(&mut post, [16, 16]);
    }
    let recorded = checked(&dir);
    // The first three frames after the start; the fourth isn't written.
    assert_eq!(
        recorded
            .iter()
            .map(|f| (f.seq, f.start))
            .collect::<Vec<_>>(),
        vec![(1, Start::First), (2, Start::Steady), (3, Start::Steady)]
    );
    // A steady frame: every buffer starts on A, so draws write B and the next reads it there.
    let keep = Use::Attach(Load::Keep);
    let white = sunrise_frame_plan::trace::ClearValue([1.0f32.to_bits(); 4]);
    let steady = units(&recorded[2]);
    let unit = |name: &str| {
        steady
            .iter()
            .find(|(label, _)| label == name)
            .unwrap_or_else(|| panic!("no unit {name}: {steady:?}"))
            .1
            .clone()
    };
    let named = |uses: &[(&str, Use)]| -> Vec<(String, Use)> {
        uses.iter().map(|(t, u)| ((*t).to_owned(), *u)).collect()
    };
    assert_eq!(
        steady.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(),
        [
            "clear",
            "world blit",
            "composite",
            "composite1",
            "composite2",
            "final",
            "centre depth",
            "centre depth readback"
        ]
    );
    assert_eq!(unit("composite"), named(&[("colortex1 B", keep)]));
    assert_eq!(
        unit("composite1"),
        named(&[("colortex2 B", keep), ("colortex1 B", Use::Sample)])
    );
    assert_eq!(unit("composite2"), named(&[("colortex3 B", keep)]));
    assert!(unit("clear").contains(&("colortex1 B".to_owned(), Use::Clear(white))));
    assert!(unit("final").contains(&("colortex2 B".to_owned(), Use::Sample)));
    // The world blit draws colortex0 and only clears the A sides of the others as it begins.
    assert!(unit("world blit").contains(&(
        "colortex3 A".to_owned(),
        Use::Clear(sunrise_frame_plan::trace::ClearValue([0; 4]))
    )));
    // composite2's colortex3 is cleared every frame and read by nothing: the draw goes (R1) in every state, and
    // both of colortex3's sides (16 × 16 RGBA8, 1024 bytes each) are left only their clears (R2): B the shared
    // clear's, A the world blit's clear-only attachment.
    let report = measure(&recorded, Callbacks::Narrowed).expect("measured");
    assert_eq!(report.sizes.len(), 1);
    let sized = &report.sizes[0];
    assert!(sized.frames.iter().all(|f| f.blocks.is_ok()));
    assert!(sized.states.iter().all(|(_, edits)| edits.is_ok()));
    assert!(!sized.removed.is_empty());
    assert!(sized.removed.iter().all(|r| r.label == "composite2"));
    let mut dead: Vec<_> = sized
        .dead
        .iter()
        .map(|d| (d.label.as_str(), d.bytes))
        .collect();
    dead.sort();
    assert_eq!(dead, [("colortex3 A", 1024), ("colortex3 B", 1024)]);
    assert_eq!(sized.dead_bytes, 2048);
    // The after-level callback may reach every side: as declared, nothing is planned.
    let declared = measure(&recorded, Callbacks::Unresolved).expect("measured");
    assert_eq!(declared.sizes[0].savings, Default::default());
}

#[test]
fn the_centre_depth_readback_is_recorded_only_when_it_is_encoded() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(post());
    let dir = tracing(&mut post, "centre", 6);
    // Nothing collects the readbacks here: after its three slots are used, the frame draws no centre depth.
    for _ in 0..6 {
        gpu.render(&mut post, [16, 16]);
    }
    let recorded = checked(&dir);
    let readbacks: Vec<(usize, usize)> = recorded
        .iter()
        .map(|f| {
            let units = units(f);
            let count = |name| units.iter().filter(|(l, _)| l == name).count();
            (count("centre depth"), count("centre depth readback"))
        })
        .collect();
    assert_eq!(readbacks, [(1, 1), (1, 1), (1, 1), (0, 0), (0, 0), (0, 0)]);
}

#[test]
fn shadow_clears_are_recorded_one_pass_each_and_none_when_nothing_is_cleared() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(pack(Coverage::Full, true, false, None, true));
    let dir = tracing(&mut post, "shadow", 1);
    gpu.render(&mut post, [16, 16]);
    let recorded = checked(&dir);
    let frame = &recorded[0];
    let texture_label = |id| {
        frame
            .textures
            .iter()
            .find(|n| n.id == id)
            .map_or("?".to_owned(), |n| n.label.clone())
    };
    let clears: Vec<Vec<String>> = frame
        .frame
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Run { work, touches, .. } if label(frame, *work) == "shadow clear" => {
                Some(touches.iter().map(|t| texture_label(t.texture)).collect())
            }
            _ => None,
        })
        .collect();
    // No shadow program draws, so the clear of what no pass cleared clears both cleared shadowcolor buffers and
    // the map: one pass per buffer, the map with the first. The images' clear before it clears nothing and is
    // no pass.
    assert_eq!(
        clears,
        [
            vec!["shadowcolor0 A".to_owned(), "shadowtex0".to_owned()],
            vec!["shadowcolor1 A".to_owned()]
        ]
    );
}

/// A `gbuffers_water` that reads `depthtex1` and writes colortex0, and a `final` that shows it; nothing draws
/// opaque terrain.
fn water_only() -> Vec<(String, String)> {
    vec![
        ("final.vsh".into(), super::depth_test::VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
             void main() { gl_FragColor = texture2D(colortex0, uv); }\n"
                .into(),
        ),
        (
            "gbuffers_water.vsh".into(),
            "#version 120\nvoid main() { gl_Position = vec4(gl_Vertex.xy, 0.0, 1.0); }\n".into(),
        ),
        (
            "gbuffers_water.fsh".into(),
            "#version 120\n/* RENDERTARGETS: 0 */\nuniform sampler2D depthtex1;\n\
             void main() { gl_FragData[0] = vec4(texture2D(depthtex1, vec2(0.5)).r); }\n"
                .into(),
        ),
    ]
}

#[test]
fn a_water_only_pack_s_opaque_block_declares_both_ends_of_its_depth_snapshot() {
    use super::depth_test::{Gpu, SIZE, fixture_frame};
    use wgpu::TextureFormat as T;
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else { return };
    const ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0=>Float32x3];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        water_only(),
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        sunrise_shaders::GameInputs {
            terrain: Some(sunrise_shaders::GbuffersInputs {
                declarations: "layout(location=0)in vec3 fixture_position;\n".into(),
                body: "hz_Vertex=vec4(fixture_position,1.0);\n".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert!(post.has_water() && post.depth1, "{:?}", post.notes);
    let dir = tracing(&mut post, "water", 1);
    let hud = HudBatch::scratch(SIZE);
    // The fixture's frame: no world callback is handed the pack's targets.
    gpu.render(
        &mut post,
        &color,
        &depth,
        &fixture_frame(&hud),
        &mut |_, _, _, _| {},
    );
    let recorded = checked(&dir);
    let frame = &recorded[0];
    let named = |label: &str| {
        frame
            .textures
            .iter()
            .find(|n| n.label == label)
            .unwrap_or_else(|| panic!("no {label}"))
            .id
    };
    let opaque = frame
        .blocks
        .iter()
        .find(|b| b.label == "opaque gbuffers")
        .expect("an opaque block");
    for end in ["depthtex0", "depthtex1"] {
        assert!(
            opaque.declared.iter().any(|s| s.texture == named(end)),
            "{end}"
        );
    }
}

/// One frame of `post` at 16×16 with the world callbacks `boundaries` hands it, each target-bearing one returning
/// `drew`.
fn render_with_callbacks(
    gpu: &Fixture,
    post: &mut PackPost,
    boundaries: WorldBoundaries,
    drew: bool,
) {
    let texture = |format| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let (world, depth, screen) = (
        texture(wgpu::TextureFormat::Rgba8Unorm),
        texture(wgpu::TextureFormat::Depth32Float),
        texture(wgpu::TextureFormat::Rgba8Unorm),
    );
    let hud = HudBatch::scratch([16, 16]);
    let frame: Frame<'_> = super::depth_test::fixture_frame(&hud);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    post.run(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &world,
        &depth,
        &screen,
        &frame,
        &Default::default(),
        [16, 16],
        [16, 16],
        &[],
        &mut |_, _, _, _| {},
        boundaries,
        |_, boundary| {
            drew && !matches!(
                boundary,
                WorldBoundary::AfterSky | WorldBoundary::AfterLevel
            )
        },
        &std::sync::atomic::AtomicBool::new(false),
        super::super::in_flight::poll_live,
    )
    .unwrap();
    gpu.queue.submit([encoder.finish()]);
}

#[test]
fn each_world_callback_s_result_is_recorded() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let mut post = gpu.build(post());
    let dir = tracing(&mut post, "callbacks", 3);
    let handed = WorldBoundaries {
        opaque_blocks: true,
        ..Default::default()
    };
    // Without a target-bearing callback, after one that drew nothing, after one that drew.
    render_with_callbacks(&gpu, &mut post, WorldBoundaries::default(), true);
    render_with_callbacks(&gpu, &mut post, handed, false);
    render_with_callbacks(&gpu, &mut post, handed, true);
    let recorded = checked(&dir);
    assert_eq!(
        recorded
            .iter()
            .map(|f| guard(f, CALLBACK_DREW))
            .collect::<Vec<_>>(),
        [Some(false), Some(false), Some(true)]
    );
    assert!(
        recorded
            .iter()
            .all(|f| guard(f, DEFERRED_SKIPPED_THEN_RAN) == Some(false))
    );
}
