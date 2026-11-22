use super::*;

#[test]
fn unsupported_defaults_are_notes_without_skipping_render_or_compute() {
    let pack = Pack::from_zip_bytes(
        "original-default-control",
        crate::pack::tests::zip(&[
            ("shaders/composite.vsh", "#version 430\nvoid main() { gl_Position = vec4(0.0); }"),
            ("shaders/composite.fsh", "#version 430\nuniform double unknown = double(2.0);\nout vec4 color; void main() { color = vec4(1.0); }"),
            ("shaders/composite.csh", "#version 430\nlayout(local_size_x=1) in; uniform double unknown = double(2.0); void main() {}"),
        ]),
    ).unwrap();
    let post = load_post(
        &pack,
        "",
        &Properties::default(),
        &Macros::standard(),
        &GameInputs::default(),
    );
    let program = post
        .programs
        .iter()
        .find(|p| p.name == "composite")
        .unwrap();
    assert!(program.program.is_some());
    assert_eq!(program.compute.len(), 1);
    for file in ["composite.fsh", "composite.csh"] {
        assert!(
            post.notes
                .iter()
                .any(|n| n.contains(file) && n.contains("unknown") && n.contains("using zero")),
            "{:?}",
            post.notes
        );
    }
    assert!(
        !post.notes.iter().any(|n| n.contains("skipped")),
        "{:?}",
        post.notes
    );
}

/// #10455, with probe packs of ours in Iris 1.11.7: a pack (or the dimension folder it
/// uses) with no gbuffers programs drew no terrain, solid or translucent, into any buffer or
/// depth, and the root folder's gbuffers programs didn't draw the Nether's terrain when
/// `world-1` had none. A `gbuffers_water` alone draws translucent terrain only (Sunrise's
/// choice; not measured in Iris).
#[test]
fn a_folder_without_terrain_programs_draws_no_terrain() {
    const VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }";
    const FSH: &str =
        "#version 120\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragData[0] = vec4(1.0); }";
    let pack = Pack::from_zip_bytes(
        "original-undrawn-terrain",
        crate::pack::tests::zip(&[
            ("shaders/gbuffers_basic.vsh", VSH),
            ("shaders/gbuffers_basic.fsh", FSH),
            ("shaders/world-1/composite.vsh", VSH),
            ("shaders/world-1/composite.fsh", FSH),
            ("shaders/world1/gbuffers_water.vsh", VSH),
            ("shaders/world1/gbuffers_water.fsh", FSH),
        ]),
    )
    .unwrap();
    let inputs = GameInputs {
        terrain: Some(GbuffersInputs::default()),
        ..Default::default()
    };
    let load = |folder: &str| {
        let post = load_post(
            &pack,
            folder,
            &Properties::default(),
            &Macros::standard(),
            &inputs,
        );
        (
            post.terrain.is_some(),
            post.water.is_some(),
            post.terrain_undrawn,
            post.water_undrawn,
        )
    };
    assert_eq!(load(""), (true, true, false, false));
    assert_eq!(load("world-1"), (false, false, true, true));
    assert_eq!(load("world1"), (false, true, true, false));
}

#[test]
fn named_dimension_folder_reaches_the_program_loader() {
    let pack = Pack::from_zip_bytes(
        "original-dimension-control",
        crate::pack::tests::zip(&[
            (
                "shaders/dimension.properties",
                "dimension.netherShaders = minecraft:the_nether\n",
            ),
            (
                "shaders/final.vsh",
                "#version 120\nvoid main() { gl_Position = gl_Vertex; }",
            ),
            (
                "shaders/final.fsh",
                "#version 120\n/* DRAWBUFFERS:0 */\nvoid main() { gl_FragColor = vec4(0.0); }",
            ),
            (
                "shaders/netherShaders/final.vsh",
                "#version 120\nvoid main() { gl_Position = gl_Vertex; }",
            ),
            (
                "shaders/netherShaders/final.fsh",
                "#version 120\n/* DRAWBUFFERS:3 */\nvoid main() { gl_FragColor = vec4(1.0); }",
            ),
        ]),
    )
    .unwrap();
    let folder = dimension_folder_named(&pack, "minecraft:the_nether", -1);
    assert_eq!(folder, "netherShaders");
    let post = load_post(
        &pack,
        &folder,
        &Properties::default(),
        &Macros::standard(),
        &GameInputs::default(),
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.programs.len(), 1);
    assert_eq!(post.programs[0].buffers, [3]);
}

#[test]
fn named_dimensions_precede_wildcards_and_legacy_folders() {
    for mappings in [
        "dimension.other = *\ndimension.specific = the_nether example:moon\n",
        "dimension.specific = the_nether example:moon\ndimension.other = *\n",
    ] {
        let pack = Pack::from_zip_bytes(
            "t",
            crate::pack::tests::zip(&[
                ("shaders/dimension.properties", mappings),
                ("shaders/world-1/final.fsh", "legacy"),
            ]),
        )
        .unwrap();
        assert_eq!(dimension_folder(&pack, -1), "specific");
        assert_eq!(dimension_folder_named(&pack, "example:moon", 0), "specific");
        assert_eq!(
            dimension_folder_named(&pack, "example:the_nether", -1),
            "other"
        );
        assert_eq!(dimension_folder(&pack, 1), "other");
    }
    let pack = Pack::from_zip_bytes(
        "t",
        crate::pack::tests::zip(&[
            (
                "shaders/dimension.properties",
                "dimension.specific = the_end\ndimension.../../escape = *\n",
            ),
            ("shaders/world-1/final.fsh", "legacy"),
        ]),
    )
    .unwrap();
    assert_eq!(dimension_folder(&pack, -1), "");
    assert_eq!(dimension_folder(&pack, 1), "specific");
    let legacy = Pack::from_zip_bytes(
        "t",
        crate::pack::tests::zip(&[("shaders/world-1/final.fsh", "legacy")]),
    )
    .unwrap();
    assert_eq!(dimension_folder(&legacy, -1), "world-1");
    assert_eq!(dimension_folder(&legacy, 0), "");
}

#[test]
fn dimension_mapping_paths_and_unknown_legacy_ids() {
    let mapped = |rules: &str| {
        Pack::from_zip_bytes(
            "t",
            crate::pack::tests::zip(&[("shaders/dimension.properties", rules)]),
        )
        .unwrap()
    };
    let pack = mapped("dimension.over = minecraft:overworld\ndimension.nested/./fallback = *\n");
    assert_eq!(dimension_folder(&pack, 0), "over");
    assert_eq!(dimension_folder(&pack, 2), "nested/fallback");
    assert_eq!(
        dimension_folder_named(&pack, "mod:moon", 2),
        "nested/fallback"
    );
    let pack = mapped("dimension.over = overworld\ndimension.../escape = *\n");
    assert_eq!(dimension_folder(&pack, 2), "");
    let pack = mapped("dimension.over = overworld\n");
    assert_eq!(dimension_folder(&pack, 2), "");
}

#[test]
fn image_dimensions_expand_default_and_chosen_options() {
    let bytes = crate::pack::tests::zip(&[("shaders/options.glsl", "#define SIZE 16 // [8 16]\n")]);
    for (choice, expected) in [(None, [16, 16]), (Some("8"), [8, 8])] {
        let mut pack = Pack::from_zip_bytes("image.zip", bytes.clone()).unwrap();
        if let Some(value) = choice {
            pack = pack.with_chosen(std::collections::BTreeMap::from([(
                "SIZE".into(),
                value.into(),
            )]));
        }
        let macros = Macros::standard();
        let properties = Properties::parse(
            "image.grid = none RGBA RGBA8 BYTE false false SIZE SIZE",
            &macros,
        );
        let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
        assert!(post.notes.is_empty(), "{:?}", post.notes);
        assert_eq!(post.images.len(), 1);
        assert_eq!(post.images[0].size, ImageSize::Absolute(expected));
    }
}

#[test]
fn image_dimensions_keep_literals_and_reject_unknown_macros() {
    let pack = Pack::from_zip_bytes(
        "image.zip",
        crate::pack::tests::zip(&[("shaders/placeholder.glsl", "")]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse(
        "image.grid = none RGBA RGBA8 BYTE false false 8 16",
        &macros,
    );
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.images[0].size, ImageSize::Absolute([8, 16]));
    let properties = Properties::parse(
        "image.grid = none RGBA RGBA8 BYTE false false UNKNOWN UNKNOWN",
        &macros,
    );
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.images.is_empty());
    assert!(!post.notes.is_empty());
}
use crate::pack::tests::zip;

const VSH: &str = "#version 120\nvarying vec2 uv;\nvoid main() {\n\
                   gl_Position = ftransform();\nuv = gl_MultiTexCoord0.xy;\n}\n";
const FSH: &str = "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\n\
                   void main() {\n/* DRAWBUFFERS:3 */\n\
                   gl_FragData[0] = texture2D(colortex0, uv);\n}\n";

#[test]
fn a_buffer_object_can_name_the_file_it_starts_with() {
    assert_eq!(
        BufferObject::parse("16 data/seed.bin"),
        Some(BufferObject::File {
            bytes: 16,
            path: "data/seed.bin".to_owned(),
            data: Vec::new(),
        })
    );
    // The other forms still read as they did.
    assert_eq!(BufferObject::parse("4096"), Some(BufferObject::Bytes(4096)));
    assert_eq!(
        BufferObject::parse("16 true 0.5 0.25"),
        Some(BufferObject::PerPixel {
            bytes: 16,
            scale: [0.5, 0.25]
        })
    );
}

#[test]
fn a_buffer_object_reads_the_file_it_names() {
    let seed: Vec<u8> = (0..16).collect();
    let pack = Pack::from_zip_bytes(
        "buffer.zip",
        crate::pack::tests::zip(&[
            (
                "shaders/shaders.properties",
                b"bufferObject.2 = 16 data/seed.bin".to_vec(),
            ),
            ("shaders/data/seed.bin", seed.clone()),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse("bufferObject.2 = 16 data/seed.bin", &macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let buffer = post.buffer_objects.get(&2).expect("the buffer is there");
    assert_eq!(buffer.initial_data(), Some(seed.as_slice()));
    assert_eq!(buffer.size([1, 1]), 16);
}

#[test]
fn a_buffer_object_whose_file_is_missing_is_refused() {
    let pack = Pack::from_zip_bytes(
        "buffer.zip",
        crate::pack::tests::zip(&[(
            "shaders/shaders.properties",
            b"bufferObject.2 = 16 data/seed.bin".to_vec(),
        )]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse("bufferObject.2 = 16 data/seed.bin", &macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.buffer_objects.is_empty());
    assert_eq!(post.notes.len(), 1, "{:?}", post.notes);
    assert!(post.notes[0].contains("data/seed.bin"), "{:?}", post.notes);
}

#[test]
fn a_buffer_object_whose_file_is_bigger_than_it_is_refused() {
    let pack = Pack::from_zip_bytes(
        "buffer.zip",
        crate::pack::tests::zip(&[
            (
                "shaders/shaders.properties",
                b"bufferObject.2 = 4 data/seed.bin".to_vec(),
            ),
            ("shaders/data/seed.bin", (0..16).collect::<Vec<u8>>()),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse("bufferObject.2 = 4 data/seed.bin", &macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.buffer_objects.is_empty());
    assert_eq!(post.notes.len(), 1, "{:?}", post.notes);
    assert!(post.notes[0].contains("bigger"), "{:?}", post.notes);
}

#[test]
fn a_buffer_object_whose_file_leaves_the_pack_is_refused() {
    let pack = Pack::from_zip_bytes(
        "buffer.zip",
        crate::pack::tests::zip(&[(
            "shaders/shaders.properties",
            "bufferObject.2 = 16 ../seed.bin",
        )]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse("bufferObject.2 = 16 ../seed.bin", &macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.buffer_objects.is_empty());
    assert_eq!(post.notes.len(), 1, "{:?}", post.notes);
}

#[test]
fn buffer_objects_and_images_parse() {
    assert_eq!(BufferObject::parse("4096"), Some(BufferObject::Bytes(4096)));
    assert_eq!(
        BufferObject::parse("64 false 1.0 1.0"),
        Some(BufferObject::Bytes(64))
    );
    assert_eq!(
        BufferObject::parse("16 true 0.5 0.25"),
        Some(BufferObject::PerPixel {
            bytes: 16,
            scale: [0.5, 0.25]
        })
    );
    assert_eq!(BufferObject::parse("16 true 0.5"), None);
    assert_eq!(BufferObject::parse("many"), None);
    let voxels = CustomImage::parse(
        "voxels",
        "voxelSampler RED_INTEGER R32UI UNSIGNED_INT true false 64 32 16",
    )
    .unwrap();
    assert_eq!(voxels.sampler.as_deref(), Some("voxelSampler"));
    assert_eq!(voxels.size, ImageSize::Volume([64, 32, 16]));
    assert!(voxels.clear);
    let half =
        CustomImage::parse("half", "none RGBA RGBA16F HALF_FLOAT false true 0.5 0.5").unwrap();
    assert_eq!(
        (half.sampler, half.size),
        (None, ImageSize::Relative([0.5, 0.5]))
    );
    assert!(!half.clear);
    assert!(CustomImage::parse("bad", "none RGBA RGBA17Q HALF_FLOAT false true 0.5 0.5").is_none());
    assert!(CustomImage::parse("short", "none RGBA RGBA16F HALF_FLOAT false").is_none());
    let flat = CustomImage::parse("flat", "none RGBA RGBA8 BYTE false false 8 8 1").unwrap();
    assert!(flat.is_3d());
    assert_eq!(flat.extent([1, 1]), [8, 8, 1]);
    assert!(
        !CustomImage::parse("plain", "none RGBA RGBA8 BYTE false false 8 8")
            .unwrap()
            .is_3d()
    );
    // One size: a 1D image, a table read by index through its sampler.
    let table = CustomImage::parse(
        "imgTable",
        "texTable RED_INTEGER R32UI UNSIGNED_INT false false 4096",
    )
    .unwrap();
    assert_eq!(table.sampler.as_deref(), Some("texTable"));
    assert_eq!(table.size, ImageSize::Line(4096));
    assert!(table.is_1d() && !table.is_3d());
    assert_eq!(table.extent([1920, 1080]), [4096, 1, 1]);
    assert!(!flat.is_1d());
    // A relative size needs both.
    assert!(CustomImage::parse("r", "none RGBA RGBA8 BYTE false true 0.5").is_none());
}

/// A pack's 1D image and its sampler: the image is loaded, its `uimage1D` declaration
/// gets the image's format, and the program reading both translates.
#[test]
fn a_1d_image_and_its_sampler_load() {
    let properties = "image.imgTable = texTable RED_INTEGER R32UI UNSIGNED_INT false false 64\n";
    let csh = "#version 430\nlayout(local_size_x = 64) in;\nwriteonly uniform uimage1D imgTable;\n\
               void main() { imageStore(imgTable, int(gl_GlobalInvocationID.x), uvec4(gl_GlobalInvocationID.x)); }\n";
    let fsh = "#version 430\nuniform usampler1D texTable;\nuniform int blockId;\n\
               /* RENDERTARGETS: 0 */\nlayout(location = 0) out vec4 color;\n\
               void main() { color = vec4(texelFetch(texTable, blockId % 64, 0).r); }\n";
    let vsh =
        "#version 430\nout vec2 uv;\nvoid main() { uv = vec2(0.0); gl_Position = ftransform(); }\n";
    let pack = Pack::from_zip_bytes(
        "table.zip",
        crate::pack::tests::zip(&[
            ("shaders/shaders.properties", properties),
            ("shaders/setup.csh", csh),
            ("shaders/composite.vsh", vsh),
            ("shaders/composite.fsh", fsh),
        ]),
    )
    .unwrap();
    let mut macros = Macros::standard();
    let (properties, notes) = crate::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.images.len(), 1);
    assert!(post.images[0].is_1d());
    let composite = post
        .programs
        .iter()
        .find(|p| p.name == "composite")
        .unwrap();
    let program = composite.program.as_ref().unwrap();
    let sampler = program
        .interface
        .samplers
        .iter()
        .find(|s| s.name == "texTable")
        .unwrap();
    assert_eq!(sampler.ty, "usampler1D");
}

/// A storage buffer declared smaller than the block a stage binds it as: the block's size
/// as the stage lays it out is what the buffer needs (std430 rounds a block to its
/// largest member's alignment), the most of every stage and program that binds it.
#[test]
fn storage_buffers_need_their_blocks_size() {
    let compute = "#version 430\nlayout(local_size_x = 1) in;\n\
                   layout(std430, binding = 0) buffer Sums { vec4 a; vec4 b; float c; } sums;\n\
                   layout(std430, binding = 1) buffer Cells { uint count; uvec4 cells[3]; } grid;\n\
                   void main() {\n    sums.c = sums.a.x + float(grid.count + grid.cells[0].x);\n}\n";
    let fsh = "#version 430\nlayout(std430, binding = 0) buffer Sums { vec4 a; } sums;\n\
               uniform sampler2D colortex0;\nvarying vec2 uv;\n\
               void main() {\n/* DRAWBUFFERS:3 */\n\
               gl_FragData[0] = texture2D(colortex0, uv) + sums.a;\n}\n";
    let pack = Pack::from_zip_bytes(
        "t",
        zip(&[
            ("shaders/composite.vsh", VSH),
            ("shaders/composite.fsh", fsh),
            ("shaders/composite.csh", compute),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse("bufferObject.0 = 40\nbufferObject.1 = 4\n", &macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.programs[0].compute.len(), 1);
    // `Sums`: 36 bytes rounded to vec4's 16; `Cells`: a uint padded to the array's 16,
    // then three elements.
    assert_eq!(
        post.buffer_minimums(),
        std::collections::BTreeMap::from([(0, 48), (1, 64)])
    );
}

#[test]
fn buffer_sizes_are_bounded() {
    let relative = BufferSize::parse("0.5 0.25").unwrap();
    assert_eq!(relative.resolve([1920, 1080]), [960, 270]);
    assert_eq!(
        BufferSize::parse("1e30 1.0").unwrap().resolve([1920, 1080])[0],
        BufferSize::MAX
    );
    assert_eq!(BufferSize::parse("inf 1.0"), None);
    assert_eq!(BufferSize::parse("-0.5 1.0"), None);
    assert_eq!(
        BufferSize::parse("100000 0").unwrap().resolve([1, 1]),
        [BufferSize::MAX, 1]
    );
}

/// `blend.<program>` blends every buffer a composite-style program writes, while
/// `blend.<program>.<buffer>` does not apply to them (Iris 1.11.7, probed: a buffer's `off`
/// under a program blend still blends, and a buffer's factors alone do not).
#[test]
fn composite_programs_blend_as_their_program_says() {
    let pack = Pack::from_zip_bytes(
        "t",
        zip(&[
            ("shaders/deferred.vsh", VSH),
            ("shaders/deferred.fsh", FSH),
            ("shaders/composite.vsh", VSH),
            ("shaders/composite.fsh", FSH),
            ("shaders/composite1.vsh", VSH),
            ("shaders/composite1.fsh", FSH),
            ("shaders/composite2.vsh", VSH),
            ("shaders/composite2.fsh", FSH),
        ]),
    )
    .unwrap();
    let properties = Properties::parse(
        "blend.deferred = ONE ONE ZERO ONE
blend.composite = SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ZERO
\
         blend.composite.colortex3 = off
blend.composite1.colortex3 = ONE ONE ONE ONE
\
         blend.composite2 = off
",
        &Macros::default(),
    );
    let post = load_post(
        &pack,
        "",
        &properties,
        &Macros::standard(),
        &GameInputs::default(),
    );
    use Factor::*;
    let blends: Vec<(&str, Option<Blend>, usize)> = post
        .programs
        .iter()
        .map(|p| (p.name.as_str(), p.blend, p.buffer_blends.len()))
        .collect();
    assert_eq!(
        blends,
        [
            ("deferred", Some([One, One, Zero, One]), 0),
            (
                "composite",
                Some([SrcAlpha, OneMinusSrcAlpha, One, Zero]),
                0
            ),
            ("composite1", None, 0),
            ("composite2", None, 0),
        ]
    );
}

#[test]
fn programs_load_in_order_with_their_settings() {
    let pack = Pack::from_zip_bytes(
        "t",
        zip(&[
            ("shaders/composite.vsh", VSH),
            ("shaders/composite.fsh", FSH),
            ("shaders/composite2.vsh", VSH),
            ("shaders/composite2.fsh", FSH),
            ("shaders/deferred.vsh", VSH),
            (
                "shaders/deferred.fsh",
                "#version 120\nconst bool colortex5MipmapEnabled = true;\n\
                 void main() { gl_FragColor = vec4(1.0); }\n",
            ),
            ("shaders/composite1.fsh", FSH),
            ("shaders/composite3.vsh", VSH),
            ("shaders/composite3.fsh", FSH),
            ("shaders/composite4.vsh", VSH),
            ("shaders/composite4.fsh", FSH),
            ("shaders/composite4.gsh", "x"),
            ("shaders/composite6.vsh", VSH),
            ("shaders/composite6.fsh", FSH),
            ("shaders/composite6.tcs", "x"),
            ("shaders/composite5.vsh", VSH),
            (
                "shaders/composite5.fsh",
                "#version 120\nvoid main() { undefined(); }\n",
            ),
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
            ("shaders/world-1/final.vsh", VSH),
            ("shaders/world-1/final.fsh", FSH),
        ]),
    )
    .unwrap();
    let properties = Properties::parse(
        "program.composite3.enabled = HQ && !LOW\nflip.composite2.gaux1=true\n\
         flip.composite.colortex3 = false\nflip.composite_pre.colortex1=true\n",
        &Macros::default(),
    );
    assert_eq!(dimension_folder(&pack, 0), "");
    assert_eq!(dimension_folder(&pack, -1), "world-1");
    let post = load_post(
        &pack,
        "",
        &properties,
        &Macros::standard(),
        &GameInputs::default(),
    );
    let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["deferred", "composite", "composite2", "final"]);
    let p = &post.programs;
    assert_eq!(p[0].buffers, [0]);
    assert_eq!((p[0].mipmaps, p[1].mipmaps), (1 << 5, 0));
    assert_eq!(p[1].buffers, [3]);
    assert_eq!(p[1].flips, [(3, false)]);
    assert_eq!(p[2].flips, [(4, true)]);
    assert_eq!(post.composite_pre, [(1, true)]);
    assert_eq!(p[3].group, Group::Final);
    let notes = post.notes.join("\n");
    assert!(
        notes.contains(
            "composite4: uses a geometry stage (supported for the shadow pass's terrain and entities only)"
        ),
        "{notes}"
    );
    assert!(
        notes.contains("composite6: uses a tessellation stage (unsupported); skipped"),
        "{notes}"
    );
    assert!(notes.contains("composite5.fsh:2:"), "{notes}");
    assert!(notes.contains("composite5: skipped"), "{notes}");

    let mut macros = Macros::standard();
    macros.define("HQ", "");
    let post = load_post(
        &pack,
        "world-1",
        &properties,
        &macros,
        &GameInputs::default(),
    );
    let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["final"]);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.programs.iter().any(|p| p.name == "composite3"));
}

#[test]
fn scale_keys_give_composite_style_programs_a_viewport() {
    let names = [
        "deferred",
        "composite",
        "composite1",
        "composite2",
        "composite3",
        "final",
    ];
    let files: Vec<(String, &str)> = names
        .iter()
        .flat_map(|n| {
            [
                (format!("shaders/{n}.vsh"), VSH),
                (format!("shaders/{n}.fsh"), FSH),
            ]
        })
        .collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    let pack = Pack::from_zip_bytes("t", zip(&files)).unwrap();
    let properties = Properties::parse(
        "scale.deferred = 0.5\nscale.composite = 0.25 0.5 0.25\nscale.composite1 = 2\n\
         scale.composite2 = 0.5 0.5\nscale.final = 0.5\n",
        &Macros::default(),
    );
    let post = load_post(
        &pack,
        "",
        &properties,
        &Macros::standard(),
        &GameInputs::default(),
    );
    let scales: Vec<(&str, [f32; 3])> = post
        .programs
        .iter()
        .map(|p| (p.name.as_str(), p.scale))
        .collect();
    assert_eq!(
        scales,
        [
            ("deferred", [0.5, 0.0, 0.0]),
            ("composite", [0.25, 0.5, 0.25]),
            // Not a scale, an offset missing, a program that draws the screen: all of it.
            ("composite1", FULL),
            ("composite2", FULL),
            ("composite3", FULL),
            ("final", FULL),
        ]
    );
}

#[test]
fn image_declarations_get_their_format_once() {
    let csh = |decl: &str| {
        format!(
            "#version 430\nlayout(local_size_x = 1) in;\n{decl}\nvoid main() {{ imageStore(voxels, ivec3(0), uvec4(1u)); }}\n"
        )
    };
    let bare = csh("writeonly uniform uimage3D voxels;");
    let split = csh("layout(r32ui)\nwriteonly uniform uimage3D voxels;");
    let pack = Pack::from_zip_bytes(
        "P.zip",
        zip(&[
            ("shaders/composite.vsh", VSH),
            (
                "shaders/composite.fsh",
                "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n",
            ),
            ("shaders/composite.csh", &bare),
            ("shaders/composite_a.csh", &split),
        ]),
    )
    .unwrap();
    let properties = Properties::parse(
        "image.voxels = none RED_INTEGER R32UI UNSIGNED_INT false false 4 4 4\n",
        &Macros::standard(),
    );
    let post = load_post(
        &pack,
        "",
        &properties,
        &Macros::standard(),
        &GameInputs::default(),
    );
    let compute: Vec<&str> = post.programs[0]
        .compute
        .iter()
        .map(|c| c.0.as_str())
        .collect();
    assert_eq!(compute, ["composite", "composite_a"], "{:?}", post.notes);
}

/// Without RENDERTARGETS or DRAWBUFFERS, a program writes colortex0 only, whatever outputs it
/// assigns, while the shadow program's outputs 0 and 1 still write shadowcolor0 and 1. The
/// programs are those of probe pack p8107 run in Iris 1.11.7 (the PR has the readings).
#[test]
fn a_program_without_a_directive_writes_buffer_0_only() {
    let full = "#version 120\nvarying vec2 uv;\nvoid main() {\n\
                gl_Position = ftransform();\nuv = gl_MultiTexCoord0.xy;\n}\n";
    let deferred = "#version 120\nuniform sampler2D colortex0;\nvarying vec2 uv;\nvoid main() {\n\
                    gl_FragData[0] = texture2D(colortex0, uv);\n\
                    gl_FragData[3] = vec4(1.1875);\ngl_FragData[7] = vec4(1.4375);\n}\n";
    let composite = "#version 330 compatibility\nuniform sampler2D colortex0;\nin vec2 uv;\n\
                     void main() {\ngl_FragData[0] = texture(colortex0, uv);\n\
                     gl_FragData[4] = vec4(2.25);\ngl_FragData[5] = vec4(2.3125);\n}\n";
    let composite1 = "#version 330 compatibility\nuniform sampler2D colortex0;\nin vec2 uv;\n\
                      layout(location = 0) out vec4 o0;\nlayout(location = 2) out vec4 o2;\n\
                      void main() {\no0 = texture(colortex0, uv);\no2 = vec4(3.125);\n}\n";
    let composite3 = "#version 330 compatibility\nuniform sampler2D colortex0;\nin vec2 uv;\n\
                      /* RENDERTARGETS: 0,12 */\nvoid main() {\n\
                      gl_FragData[0] = texture(colortex0, uv);\ngl_FragData[1] = vec4(6.5);\n}\n";
    let terrain_vsh = "#version 120\nvarying vec2 texcoord;\nvarying vec4 color;\nvoid main() {\n\
                       gl_Position = ftransform();\n\
                       texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\ncolor = gl_Color;\n}\n";
    let terrain = "#version 120\nuniform sampler2D texture;\nvarying vec2 texcoord;\n\
                   varying vec4 color;\nvoid main() {\n\
                   vec4 c = texture2D(texture, texcoord) * color;\nif (c.a < 0.1) discard;\n\
                   gl_FragData[0] = c;\ngl_FragData[6] = vec4(5.0);\n}\n";
    let shadow_vsh = "#version 120\nvoid main() {\ngl_Position = ftransform();\n}\n";
    let shadow = "#version 120\nvoid main() {\n\
                  gl_FragData[0] = vec4(0.25);\ngl_FragData[1] = vec4(0.5);\n}\n";
    let prepare = "#version 120\nvoid main() {\n\
                   gl_FragData[0] = vec4(0.375);\ngl_FragData[2] = vec4(8.5);\n}\n";
    let shadowcomp = "#version 120\nvoid main() {\n\
                      gl_FragData[0] = vec4(0.75);\ngl_FragData[1] = vec4(0.875);\n}\n";
    let reads_shadow = "#version 120\nuniform sampler2D shadowcolor1;\nvarying vec2 uv;\n\
                        void main() { gl_FragColor = texture2D(shadowcolor1, uv); }\n";
    let pack = Pack::from_zip_bytes(
        "t",
        zip(&[
            ("shaders/deferred.vsh", full),
            ("shaders/deferred.fsh", deferred),
            ("shaders/composite.vsh", full),
            ("shaders/composite.fsh", composite),
            ("shaders/composite1.vsh", full),
            ("shaders/composite1.fsh", composite1),
            ("shaders/composite3.vsh", full),
            ("shaders/composite3.fsh", composite3),
            ("shaders/gbuffers_terrain.vsh", terrain_vsh),
            ("shaders/gbuffers_terrain.fsh", terrain),
            ("shaders/shadow.vsh", shadow_vsh),
            ("shaders/shadow.fsh", shadow),
            ("shaders/prepare.vsh", full),
            ("shaders/prepare.fsh", prepare),
            ("shaders/shadowcomp.vsh", full),
            ("shaders/shadowcomp.fsh", shadowcomp),
            ("shaders/final.vsh", full),
            ("shaders/final.fsh", reads_shadow),
        ]),
    )
    .unwrap();
    let game = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
        ..Default::default()
    };
    let inputs = GameInputs {
        terrain: Some(game.clone()),
        entities: Some(game),
        ..Default::default()
    };
    let post = load_post(
        &pack,
        "",
        &Properties::default(),
        &Macros::standard(),
        &inputs,
    );
    let written: Vec<(&str, &[u32])> = (post.programs.iter())
        .chain(&post.terrain)
        .chain(&post.shadow)
        .chain(&post.shadow_comp)
        .map(|p| (p.name.as_str(), &p.buffers[..]))
        .collect();
    assert_eq!(
        written,
        [
            ("prepare", &[0][..]),
            ("deferred", &[0]),
            ("composite", &[0]),
            ("composite1", &[0]),
            ("composite3", &[0, 12]),
            ("final", &[0]),
            ("gbuffers_terrain", &[0]),
            ("shadow", &[0, 1]),
            ("shadowcomp", &[0, 1]),
        ],
        "{:?}",
        post.notes
    );
}

/// The shadow pass runs when a program reads a shadow buffer or `shadow.enabled` says so;
/// then `shadow`, `shadow_water` and the `shadowcomp` series load, writing shadowcolor0/1.
#[test]
#[ignore = "daily"]
fn shadow_programs_load_when_a_shadow_buffer_is_read() {
    let reads = "#version 120\nuniform sampler2D shadowtex0;\nvarying vec2 uv;\n\
                 void main() { gl_FragColor = texture2D(shadowtex0, uv); }\n";
    let shadow = "#version 120\nvarying vec2 uv;\nvoid main() {\n/* RENDERTARGETS: 1,0,4 */\n\
                  gl_FragData[0] = vec4(1.0); gl_FragData[1] = vec4(0.5); gl_FragData[2] = vec4(0.0);\n}\n";
    let files = [
        ("shaders/composite.vsh", VSH),
        ("shaders/composite.fsh", reads),
        ("shaders/shadow.vsh", VSH),
        ("shaders/shadow.fsh", shadow),
        ("shaders/shadowcomp1.vsh", VSH),
        ("shaders/shadowcomp1.fsh", FSH),
        ("shaders/shadow_block.vsh", VSH),
        ("shaders/shadow_block.fsh", FSH),
    ];
    let pack = Pack::from_zip_bytes("t", zip(&files)).unwrap();
    let game = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n".into(),
        ..Default::default()
    };
    let inputs = GameInputs {
        terrain: Some(game.clone()),
        entities: Some(game),
        ..Default::default()
    };
    let load = |properties: &str| {
        let properties = Properties::parse(properties, &Macros::default());
        load_post(&pack, "", &properties, &Macros::standard(), &inputs)
    };
    let post = load("");
    assert!(post.shadow_pass);
    let shadow = post.shadow.as_ref().expect("shadow");
    assert_eq!(
        (shadow.group, &shadow.buffers[..]),
        (Group::Shadow, &[1, 0][..])
    );
    // shadow_water falls back to shadow.
    assert_eq!(
        post.shadow_water.as_ref().map(|p| p.name.as_str()),
        Some("shadow")
    );
    // ...with the boats' water patches, and once more without them.
    assert_eq!(
        post.shadow_water_plain.as_ref().map(|p| p.name.as_str()),
        Some("shadow")
    );
    // Entities fall back to shadow; block entities have their own program.
    let name = |p: &Option<PostProgram>| p.as_ref().map(|p| p.name.clone());
    assert_eq!(
        (name(&post.shadow_entities), name(&post.shadow_block)),
        (Some("shadow".into()), Some("shadow_block".into()))
    );
    let off = load("shadowEntities=false\nshadowBlockEntities = false");
    assert!(off.shadow.is_some() && off.shadow_entities.is_none() && off.shadow_block.is_none());
    // shadowPlayer and shadowLightBlockEntities keep a part of each.
    let some = load(
        "shadowEntities=false\nshadowPlayer=true\nshadowBlockEntities=false\nshadowLightBlockEntities=true",
    );
    assert!(some.shadow_entities.is_some() && some.shadow_only_player);
    assert!(some.shadow_block.is_some() && some.shadow_only_lit_blocks);
    assert!(!post.shadow_only_player && !post.shadow_only_lit_blocks);
    let comp: Vec<_> = post
        .shadow_comp
        .iter()
        .map(|p| (p.name.as_str(), p.group))
        .collect();
    assert_eq!(comp, [("shadowcomp1", Group::ShadowComp)]);
    // DRAWBUFFERS:3 names no shadowcolor buffer.
    assert!(post.shadow_comp[0].buffers.is_empty());
    let post = load("shadow.enabled = false");
    assert!(!post.shadow_pass && post.shadow.is_none() && post.shadow_comp.is_empty());
    // Nothing reads a shadow buffer.
    let pack = Pack::from_zip_bytes("t", zip(&files[2..])).unwrap();
    let none = |properties: &str| {
        let properties = Properties::parse(properties, &Macros::default());
        load_post(&pack, "", &properties, &Macros::standard(), &inputs)
    };
    assert!(!none("").shadow_pass);
    assert!(none("shadow.enabled=true").shadow.is_some());
    let post = none("shadow.enabled=true\nshadowTranslucent=false");
    assert!(post.shadow.is_some() && post.shadow_water.is_none());
}

/// A compute stage that samples a shadow buffer turns the shadow pass on as a fragment stage
/// does. Iris 1.11.7 with probe packs of ours (#10490), where nothing else read them: a
/// `prepare.csh` alone sampling shadowtex0 read the terrain's depth there (0.3956 at our noon
/// scene), a `composite.csh` beside a `composite.fsh` that reads none read the same, and a
/// `prepare.csh` sampling shadowcolor0 read what `shadow.fsh` wrote (0.251); Sunrise read an
/// empty map (1.0) in all three.
#[test]
fn a_compute_stage_reading_a_shadow_buffer_turns_the_shadow_pass_on() {
    let csh = |sampler: &str| {
        format!(
            "#version 430\nlayout(local_size_x = 1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
             uniform sampler2D {sampler};\nlayout(rgba8) writeonly uniform image2D colorimg5;\n\
             void main() {{ imageStore(colorimg5, ivec2(0), texture({sampler}, vec2(0.5))); }}\n"
        )
    };
    let load = |files: &[(&str, &str)]| {
        let mut files = files.to_vec();
        files.extend([
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
            ("shaders/shadow.vsh", VSH),
            ("shaders/shadow.fsh", FSH),
        ]);
        let pack = Pack::from_zip_bytes("t", zip(&files)).unwrap();
        let properties = Properties::parse("", &Macros::default());
        load_post(
            &pack,
            "",
            &properties,
            &Macros::standard(),
            &GameInputs::default(),
        )
    };
    let prepare = csh("shadowtex0");
    let post = load(&[("shaders/prepare.csh", &prepare)]);
    assert!(
        post.programs.iter().any(|p| p.name == "prepare"),
        "{:?}",
        post.notes
    );
    assert!(post.shadow_pass, "{:?}", post.notes);
    let post = load(&[
        ("shaders/composite.vsh", VSH),
        ("shaders/composite.fsh", FSH),
        ("shaders/composite.csh", &prepare),
    ]);
    let composite = post.programs.iter().find(|p| p.name == "composite");
    assert_eq!(composite.map(|p| p.compute.len()), Some(1));
    assert!(post.shadow_pass, "{:?}", post.notes);
    let color = csh("shadowcolor0");
    assert!(load(&[("shaders/prepare.csh", &color)]).shadow_pass);
    // A compute stage that reads none leaves it off, as before.
    let colortex = csh("colortex1");
    assert!(!load(&[("shaders/prepare.csh", &colortex)]).shadow_pass);
}

/// `HIGHER_SHADOWCOLOR` (required or optional) gives the shadow programs shadowcolor2 to 7,
/// with their own settings; a directive reaches them, no directive still writes 0 and 1.
/// Without the feature, outputs stop at shadowcolor1 as before. As in Iris 1.11.7 with
/// probe packs p7951a to c (the PR has the readings).
#[test]
#[ignore = "daily"]
fn higher_shadowcolor_gives_shadow_programs_eight_buffers() {
    let shadow_vsh = "#version 120\nvoid main() {\ngl_Position = ftransform();\n}\n";
    let writes = |targets: Option<&str>| {
        let directive = targets.map_or(String::new(), |t| format!("/* RENDERTARGETS: {t} */\n"));
        format!(
            "#version 120\nconst vec4 shadowcolor5ClearColor = vec4(0.3, 0.4, 0.5, 0.6);\n\
             /* const int shadowcolor6Format = RGBA32F; */\n{directive}void main() {{\n\
             gl_FragData[0] = vec4(0.0625);\ngl_FragData[1] = vec4(0.125);\n\
             gl_FragData[2] = vec4(0.1875);\ngl_FragData[3] = vec4(0.25);\n\
             gl_FragData[4] = vec4(0.3125);\n}}\n"
        )
    };
    let full = "#version 120\nvarying vec2 uv;\nvoid main() {\n\
                gl_Position = ftransform();\nuv = gl_MultiTexCoord0.xy;\n}\n";
    let shadowcomp = "#version 120\n/* RENDERTARGETS: 7 */\n\
                      void main() {\ngl_FragData[0] = vec4(0.9375);\n}\n";
    let reads = "#version 120\nuniform sampler2D shadowcolor7;\nvarying vec2 uv;\n\
                 void main() { gl_FragColor = texture2D(shadowcolor7, uv); }\n";
    let game = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n".into(),
        ..Default::default()
    };
    let inputs = GameInputs {
        terrain: Some(game.clone()),
        entities: Some(game),
        ..Default::default()
    };
    let load = |properties: &str, targets: Option<&str>| {
        let shadow = writes(targets);
        let pack = Pack::from_zip_bytes(
            "t",
            zip(&[
                ("shaders/shadow.vsh", shadow_vsh),
                ("shaders/shadow.fsh", &shadow),
                ("shaders/shadowcomp.vsh", full),
                ("shaders/shadowcomp.fsh", shadowcomp),
                ("shaders/final.vsh", full),
                ("shaders/final.fsh", reads),
            ]),
        )
        .unwrap();
        let properties = Properties::parse(properties, &Macros::default());
        load_post(&pack, "", &properties, &Macros::standard(), &inputs)
    };
    let buffers = |post: &Post| {
        let shadow = post.shadow.as_ref().map(|p| p.buffers.clone());
        let comp: Vec<_> = post.shadow_comp.iter().map(|p| p.buffers.clone()).collect();
        (post.shadow_colors, shadow, comp)
    };
    let required = "iris.features.required = HIGHER_SHADOWCOLOR\n";
    let post = load(required, Some("0,1,2,3,4"));
    // Reading shadowcolor7 alone turns the shadow pass on.
    assert!(post.shadow_pass, "{:?}", post.notes);
    assert_eq!(
        buffers(&post),
        (8, Some(vec![0, 1, 2, 3, 4]), vec![vec![7]])
    );
    let s = &post.config.shadow;
    assert_eq!(s.colors[5].clear_color, Some([0.3, 0.4, 0.5, 0.6]));
    assert_eq!(s.colors[6].format, crate::Format::parse("RGBA32F").unwrap());
    assert_eq!(s.colors[7].clear_color, Some([1.0; 4]));
    let optional = "iris.features.optional = HIGHER_SHADOWCOLOR\n";
    let post = load(optional, Some("0,1,2,3,4"));
    assert_eq!(buffers(&post).1, Some(vec![0, 1, 2, 3, 4]));
    // Iris 1.11.7 takes the name in any case (#8996, a probe pack of our own).
    let post = load(
        "iris.features.optional = higher_shadowcolor\n",
        Some("0,1,2,3,4"),
    );
    assert_eq!(buffers(&post).1, Some(vec![0, 1, 2, 3, 4]));
    // No directive: shadowcolor0 and 1, as without the feature.
    let post = load(required, None);
    assert_eq!(buffers(&post).1, Some(vec![0, 1]));
    // Without the feature: two buffers; shadowcolor7 doesn't turn the shadow pass on, and
    // with it forced on, RENDERTARGETS: 7 writes nothing.
    let post = load("", Some("0,1,2,3,4"));
    assert!(!post.shadow_pass);
    let post = load("shadow.enabled = true\n", Some("0,1,2,3,4"));
    assert_eq!(buffers(&post), (2, Some(vec![0, 1]), vec![vec![]]));
}

/// A pack without a shadow pass still takes `shadow.fsh`'s shadow settings, not
/// `shadow.vsh`'s: Iris 1.11.7 applied `shadowDistance` and `sunPathRotation` from the first
/// (shadow projection, sun position) and not from the second, with probe packs of ours whose
/// shadow buffers nothing read (#10490).
#[test]
fn shadow_fsh_settings_count_without_a_shadow_pass() {
    let settings = "const float shadowDistance = 48.0;\nconst float sunPathRotation = 20.0;\n";
    let load = |vsh: &str, fsh: &str| {
        let files = [
            ("shaders/final.vsh", VSH),
            ("shaders/final.fsh", FSH),
            ("shaders/shadow.vsh", vsh),
            ("shaders/shadow.fsh", fsh),
        ];
        let pack = Pack::from_zip_bytes("t", zip(&files)).unwrap();
        let properties = Properties::parse("", &Macros::default());
        load_post(
            &pack,
            "",
            &properties,
            &Macros::standard(),
            &GameInputs::default(),
        )
    };
    let with = |text: &str, file: &str| format!("#version 120\n{text}{}", &file[13..]);
    let post = load(VSH, &with(settings, FSH));
    assert!(!post.shadow_pass);
    let s = &post.config.shadow;
    assert_eq!((s.distance, s.sun_path_rotation), (48.0, 20.0));
    let post = load(&with(settings, VSH), FSH);
    let (s, d) = (&post.config.shadow, crate::config::Shadow::default());
    assert_eq!(
        (s.distance, s.sun_path_rotation),
        (d.distance, d.sun_path_rotation)
    );
}

/// A shadow program with a geometry stage draws terrain, entities and block entities, each
/// read vertex by vertex as the game says; a draw whose vertices can't be read so skips it.
#[test]
fn a_shadow_geometry_stage_draws_entities_and_block_entities() {
    let reads = "#version 120\nuniform sampler2D shadowtex0;\nvarying vec2 uv;\n\
                 void main() { gl_FragColor = texture2D(shadowtex0, uv); }\n";
    let vsh = "#version 330\nout vec2 vUv;\nvoid main() { gl_Position = ftransform(); vUv = vec2(0.0); }\n";
    let gsh = "#version 330\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
               in vec2 vUv[];\nout vec2 gUv;\nvoid main() {\n    for (int i = 0; i < 3; i++) {\n        \
               gl_Position = gl_in[i].gl_Position;\n        gUv = vUv[i];\n        EmitVertex();\n    }\n    \
               EndPrimitive();\n}\n";
    let fsh = "#version 330\nin vec2 gUv;\nlayout(location = 0) out vec4 c;\nvoid main() { c = vec4(gUv, 0.0, 1.0); }\n";
    let files = [
        ("shaders/composite.vsh", VSH),
        ("shaders/composite.fsh", reads),
        ("shaders/shadow.vsh", vsh),
        ("shaders/shadow.gsh", gsh),
        ("shaders/shadow.fsh", fsh),
    ];
    let pack = Pack::from_zip_bytes("t", zip(&files)).unwrap();
    let game = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;\n\
                       layout(set = 1, binding = 0) readonly buffer V { uint v[]; };\n"
            .into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n".into(),
        ..Default::default()
    };
    let geometry = crate::GeometryInputs {
        inputs: game.clone(),
        pull: "        hz_v_pos = uintBitsToFloat(uvec3(v[hz_vertex_index], v[hz_vertex_index], v[hz_vertex_index]));\n"
            .into(),
        index: "uint(@)".into(),
    };
    let mut inputs = GameInputs {
        terrain: Some(game.clone()),
        entities: Some(game),
        terrain_geometry: Some(geometry.clone()),
        entity_geometry: Some(geometry),
        ..Default::default()
    };
    let load = |inputs: &GameInputs| {
        load_post(
            &pack,
            "",
            &Properties::default(),
            &Macros::standard(),
            inputs,
        )
    };
    let post = load(&inputs);
    let triangles = |p: &Option<PostProgram>| p.as_ref().map(|p| p.geometry_triangles);
    assert_eq!(
        [&post.shadow, &post.shadow_entities, &post.shadow_block].map(triangles),
        [Some(1); 3],
        "{:?}",
        post.notes
    );
    inputs.entity_geometry = None;
    let post = load(&inputs);
    assert!(post.shadow.is_some());
    assert!(post.shadow_entities.is_none() && post.shadow_block.is_none());
    assert!(
        post.notes
            .iter()
            .any(|n| n.starts_with("shadow: uses a geometry stage")),
        "{:?}",
        post.notes
    );
}

/// Water falls back to the terrain chain and blends by default, as the sky does; entities
/// and block entities fall back along theirs; `blend.*` overrides.
/// `clouds=off` hides the game's clouds and drops the pack's own cloud program.
#[test]
fn clouds_off_draws_no_clouds() {
    let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
    let fsh = "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    let pack = Pack::from_zip_bytes(
        "clouds.zip",
        crate::pack::tests::zip(&[
            ("shaders/gbuffers_clouds.vsh", vsh),
            ("shaders/gbuffers_clouds.fsh", fsh),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let inputs = GameInputs {
        clouds: Some(crate::GbuffersInputs {
            declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
            body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let on = load_post(&pack, "", &Properties::default(), &macros, &inputs);
    assert!(on.clouds.is_some() && !on.clouds_off);
    let off = Properties::parse("clouds = off\n", &macros);
    let off = load_post(&pack, "", &off, &macros, &inputs);
    assert!(off.clouds.is_none() && off.clouds_off);
}

/// Without `gbuffers_weather` or the textured programs, rain and snow draw with
/// `gbuffers_basic`, blended, whatever `rain.depth` says.
#[test]
fn weather_falls_back_to_basic_whatever_rain_depth_says() {
    let pack = Pack::from_zip_bytes(
        "original-weather-depth",
        crate::pack::tests::zip(&[
            (
                "shaders/gbuffers_basic.vsh",
                "#version 120\nvoid main(){gl_Position=vec4(0.0,0.0,0.0,1.0);}",
            ),
            (
                "shaders/gbuffers_basic.fsh",
                "#version 120\nvoid main(){gl_FragColor=vec4(1.0);}",
            ),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let inputs = GameInputs {
        weather: Some(GbuffersInputs::default()),
        ..Default::default()
    };
    for text in ["", "rain.depth = true", "rain.depth = false"] {
        let properties = Properties::parse(text, &macros);
        let post = load_post(&pack, "", &properties, &macros, &inputs);
        let weather = post.weather.expect("weather fallback loads");
        assert_eq!(weather.name, "gbuffers_basic");
        assert_eq!(weather.group, Group::Translucent);
        assert_eq!(weather.blend, Some(TRANSLUCENT_BLEND));
    }
}

/// Without `gbuffers_clouds` the clouds draw with `gbuffers_textured`, else `gbuffers_basic`,
/// blended; a pack with none of them leaves the clouds to the game.
#[test]
fn clouds_fall_back_to_textured_then_basic() {
    let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
    let fsh = "#version 120\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    let macros = Macros::standard();
    let inputs = GameInputs {
        clouds: Some(crate::GbuffersInputs {
            declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
            body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let clouds = |programs: &[&str]| {
        let files: Vec<(String, &str)> = programs
            .iter()
            .flat_map(|p| {
                [
                    (format!("shaders/{p}.vsh"), vsh),
                    (format!("shaders/{p}.fsh"), fsh),
                ]
            })
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("clouds.zip", crate::pack::tests::zip(&files)).unwrap();
        load_post(&pack, "", &Properties::default(), &macros, &inputs)
            .clouds
            .map(|p| (p.name, p.blend))
    };
    let blended = |name: &str| Some((name.to_owned(), Some(TRANSLUCENT_BLEND)));
    assert_eq!(
        clouds(&["gbuffers_textured", "gbuffers_basic"]),
        blended("gbuffers_textured")
    );
    assert_eq!(
        clouds(&["gbuffers_textured_lit", "gbuffers_basic"]),
        blended("gbuffers_basic")
    );
    assert_eq!(
        clouds(&["gbuffers_clouds", "gbuffers_textured"]),
        blended("gbuffers_clouds")
    );
    assert_eq!(clouds(&["gbuffers_terrain", "composite"]), None);
}

const TERRAIN_VSH: &str = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
const TERRAIN_FSH: &str = "#version 120\nvoid main() { gl_FragData[0] = vec4(1.0); }\n";

/// The pack of `files` (under `shaders/`), with `properties`, loaded with the game's terrain
/// inputs and, with `parts`, its two parts.
fn terrain_files(files: &[(String, &str)], properties: &str, parts: bool) -> Post {
    let files: Vec<_> = (files.iter())
        .map(|(name, source)| (format!("shaders/{name}"), *source))
        .collect();
    let files: Vec<_> = files.iter().map(|(n, s)| (n.as_str(), *s)).collect();
    let pack = Pack::from_zip_bytes("terrain-parts", crate::pack::tests::zip(&files)).unwrap();
    let inputs = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
        ..Default::default()
    };
    let one_pass = crate::GbuffersInputs {
        parts: Some(crate::Parts {
            solid: "hz_v_pos.x > 0.0".into(),
            cutout_alpha_test: None,
        }),
        ..inputs.clone()
    };
    let inputs = GameInputs {
        terrain_parts: parts.then(|| [inputs.clone(), inputs.clone()]),
        terrain_one_pass: parts.then_some(one_pass),
        terrain: Some(inputs),
        ..Default::default()
    };
    let macros = Macros::standard();
    let properties = Properties::parse(properties, &macros);
    load_post(&pack, "", &properties, &macros, &inputs)
}

/// `terrain_files` with `programs`, each writing white.
fn terrain_pack(programs: &[&str], properties: &str, parts: bool) -> Post {
    let files: Vec<_> = (programs.iter())
        .flat_map(|p| {
            [
                (format!("{p}.vsh"), TERRAIN_VSH),
                (format!("{p}.fsh"), TERRAIN_FSH),
            ]
        })
        .collect();
    terrain_files(&files, properties, parts)
}

/// What draws solid (or all) terrain and what draws cutout terrain.
fn terrain_names(programs: &[&str], properties: &str, parts: bool) -> [Option<String>; 2] {
    let post = terrain_pack(programs, properties, parts);
    [post.terrain, post.terrain_cutout].map(|p| p.map(|p| p.name))
}

const TERRAIN_PROGRAMS: [&str; 3] = [
    "gbuffers_terrain",
    "gbuffers_terrain_solid",
    "gbuffers_terrain_cutout",
];

/// Iris 1.11.7 with our probe packs (#9137): `gbuffers_terrain_solid` draws solid terrain
/// and `gbuffers_terrain_cutout` cutout terrain, each falling back to `gbuffers_terrain`;
/// a pack with only `gbuffers_terrain` draws both parts with it, in one pass, each with its
/// own alpha test (#9307).
#[test]
fn terrain_solid_and_cutout_fall_back_to_terrain() {
    let names = |programs: &[&str], parts: bool| terrain_names(programs, "", parts);
    let some = |name: &str| Some(name.to_owned());
    let [terrain, solid, cutout] = TERRAIN_PROGRAMS;
    assert_eq!(
        names(&[terrain, solid, cutout], true),
        [some(solid), some(cutout)]
    );
    assert_eq!(names(&[terrain, solid], true), [some(solid), some(terrain)]);
    assert_eq!(
        names(&[terrain, cutout], true),
        [some(terrain), some(cutout)]
    );
    assert_eq!(names(&[terrain], true), [some(terrain), None]);
    // Without the game's two parts, terrain draws whole.
    assert_eq!(
        names(&[terrain, solid, cutout], false),
        [some(terrain), None]
    );
}

/// A pack whose only terrain program is `gbuffers_terrain_solid` (or `_cutout`) has nothing
/// for the other part: that part draws alone, and nothing draws the other, as Iris 1.11.7
/// did with our probes `tsc-solidonly` and `tsc-cutoutonly`.
#[test]
fn a_lone_split_program_draws_only_its_part() {
    let [_, solid, cutout] = TERRAIN_PROGRAMS;
    let some = |name: &str| Some(name.to_owned());
    assert_eq!(terrain_names(&[solid], "", true), [some(solid), None]);
    assert_eq!(terrain_names(&[cutout], "", true), [None, some(cutout)]);
    let post = terrain_pack(&[solid], "", true);
    let note = "gbuffers_terrain_solid: no program for cutout terrain; nothing draws it";
    assert!(post.notes.iter().any(|n| n == note), "{:?}", post.notes);
}

/// When the other part has a program that fails to load, the part that loaded doesn't draw
/// alone: the game draws all terrain (#9176's review).
#[test]
fn a_split_part_that_fails_to_load_takes_the_other_with_it() {
    let [_, solid, cutout] = TERRAIN_PROGRAMS;
    // `cutout` with a stage that doesn't translate.
    let files = [
        (format!("{solid}.vsh"), TERRAIN_VSH),
        (format!("{solid}.fsh"), TERRAIN_FSH),
        (format!("{cutout}.vsh"), TERRAIN_VSH),
        (
            format!("{cutout}.fsh"),
            "void main() { gl_FragData[0] = vec4(1.0) }\n",
        ),
    ];
    let post = terrain_files(&files, "", true);
    assert!(post.terrain.is_none() && post.terrain_cutout.is_none());
    let note =
        "gbuffers_terrain_solid: cutout terrain's program didn't load; the game draws all terrain";
    assert!(post.notes.iter().any(|n| n == note), "{:?}", post.notes);
}

/// `program.<name>.enabled` turning off the only split program leaves both parts to
/// `gbuffers_terrain`, as for a pack without it: Iris 1.11.7 drew all terrain with
/// `gbuffers_terrain` for our probe `tsc-disabled-false`.
#[test]
fn a_disabled_split_program_leaves_terrain_whole() {
    let [terrain, solid, _] = TERRAIN_PROGRAMS;
    let off = "program.gbuffers_terrain_solid.enabled = false\n";
    let some = |name: &str| Some(name.to_owned());
    assert_eq!(
        terrain_names(&[terrain, solid], off, true),
        [some(terrain), None]
    );
    let on = "program.gbuffers_terrain_solid.enabled = true\n";
    assert_eq!(
        terrain_names(&[terrain, solid], on, true),
        [some(solid), some(terrain)]
    );
}

/// The cutout part's storage blocks count towards the buffers' sizes, like every other
/// program's.
#[test]
fn the_cutout_parts_storage_blocks_size_their_buffers() {
    let fsh = "#version 430\nlayout(std430, binding = 0) buffer Sums { vec4 a[4]; } sums;\n\
               void main() {\n/* DRAWBUFFERS:0 */\ngl_FragData[0] = sums.a[3];\n}\n";
    let files = [
        ("gbuffers_terrain.vsh".into(), TERRAIN_VSH),
        ("gbuffers_terrain.fsh".into(), TERRAIN_FSH),
        ("gbuffers_terrain_cutout.vsh".into(), TERRAIN_VSH),
        ("gbuffers_terrain_cutout.fsh".into(), fsh),
    ];
    let post = terrain_files(&files, "bufferObject.0 = 16\n", true);
    let cutout = post.terrain_cutout.as_ref().map(|p| p.name.as_str());
    assert_eq!(cutout, Some("gbuffers_terrain_cutout"), "{:?}", post.notes);
    assert_eq!(
        post.buffer_minimums(),
        std::collections::BTreeMap::from([(0, 64)])
    );
}

/// A pack's own `gbuffers_hand_water` draws the translucent held items: after deferred,
/// blended, beside its opaque `gbuffers_hand`.
#[test]
fn translucent_held_items_draw_with_gbuffers_hand_water() {
    let vsh = "#version 120\nvarying vec2 uv;\nvoid main() {\n    gl_Position = ftransform();\n    uv = gl_MultiTexCoord0.xy;\n}\n";
    let fsh = "#version 120\nuniform sampler2D texture;\nvarying vec2 uv;\nvoid main() {\n    gl_FragData[0] = texture2D(texture, uv);\n}\n";
    let pack = Pack::from_zip_bytes(
        "P.zip",
        zip(&[
            ("shaders/gbuffers_hand.vsh", vsh),
            ("shaders/gbuffers_hand.fsh", fsh),
            ("shaders/gbuffers_hand_water.vsh", vsh),
            ("shaders/gbuffers_hand_water.fsh", fsh),
        ]),
    )
    .unwrap();
    let hand = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
        ..Default::default()
    };
    let inputs = GameInputs {
        hand: Some(hand),
        ..Default::default()
    };
    let post = load_post(
        &pack,
        "",
        &Properties::default(),
        &Macros::standard(),
        &inputs,
    );
    let (opaque, translucent) = (post.hand.unwrap(), post.hand_water.unwrap());
    assert_eq!(
        (opaque.name.as_str(), opaque.blend),
        ("gbuffers_hand", None)
    );
    assert_eq!(
        (
            translucent.name.as_str(),
            translucent.group,
            translucent.blend
        ),
        (
            "gbuffers_hand_water",
            Group::Translucent,
            Some(TRANSLUCENT_BLEND)
        )
    );
}

#[test]
#[ignore = "daily"]
fn water_falls_back_to_terrain_and_blends() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/terrain-test");
    let pack = Pack::open(&dir).unwrap();
    let macros = Macros::standard();
    let inputs = crate::GbuffersInputs {
        declarations: "layout(location = 0) in vec3 hz_v_pos;".into(),
        body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
        ..Default::default()
    };
    let inputs = GameInputs {
        terrain: Some(inputs.clone()),
        sky: Some(inputs.clone()),
        celestial: Some(inputs.clone()),
        entities: Some(inputs.clone()),
        clouds: Some(inputs.clone()),
        weather: Some(inputs.clone()),
        lines: Some(inputs.clone()),
        hand: Some(inputs),
        terrain_parts: None,
        terrain_one_pass: None,
        terrain_geometry: None,
        entity_geometry: None,
    };
    let post = load_post(&pack, "", &Properties::default(), &macros, &inputs);
    let (terrain, water) = (post.terrain.unwrap(), post.water.unwrap());
    assert_eq!((terrain.group, terrain.blend), (Group::Gbuffers, None));
    assert_eq!(water.name, "gbuffers_terrain");
    let sky = post.sky.unwrap();
    assert_eq!(
        (sky.name.as_str(), sky.group, sky.blend),
        (
            "gbuffers_skybasic",
            Group::Gbuffers,
            Some(TRANSLUCENT_BLEND)
        )
    );
    // Its parts blend as vanilla's: the stars with BlendFunction.OVERLAY.
    assert!(sky.vanilla_sky_blends);
    assert_eq!(
        STARS_BLEND,
        [Factor::SrcAlpha, Factor::One, Factor::One, Factor::Zero]
    );
    // gbuffers_skytextured falls back to gbuffers_textured, then gbuffers_basic.
    let sun = post.sky_textured.unwrap();
    assert_eq!(
        (sun.name.as_str(), sun.blend),
        ("gbuffers_textured", Some(CELESTIAL_BLEND))
    );
    // Vanilla's `BlendFunction.OVERLAY`: colour added, alpha the sun's own.
    assert_eq!(
        sun.blend,
        Some([Factor::SrcAlpha, Factor::One, Factor::One, Factor::Zero])
    );
    assert_eq!(
        (water.group, water.blend),
        (Group::Translucent, Some(TRANSLUCENT_BLEND))
    );
    // Entities fall back to gbuffers_textured, block entities to gbuffers_terrain; both
    // draw without blending.
    let (entities, block) = (post.entities.unwrap(), post.block.unwrap());
    assert_eq!(
        (entities.name.as_str(), entities.blend),
        ("gbuffers_textured", None)
    );
    assert_eq!(
        (block.name.as_str(), block.blend),
        ("gbuffers_terrain", None)
    );
    // Translucent entity parts and particles blend; particles draw after deferred unless
    // the pack says otherwise, translucent entities before unless `separateEntityDraws`.
    let (translucent, particles) = (
        post.entities_translucent.as_ref().unwrap(),
        post.particles.as_ref().unwrap(),
    );
    assert_eq!(
        (translucent.name.as_str(), translucent.blend),
        ("gbuffers_textured", Some(TRANSLUCENT_BLEND))
    );
    assert_eq!(particles.blend, Some(TRANSLUCENT_BLEND));
    assert!(post.particles_late && !post.translucent_late);
    // Clouds fall back to gbuffers_textured, blended.
    let clouds = post.clouds.as_ref().unwrap();
    assert_eq!(
        (clouds.name.as_str(), clouds.blend),
        ("gbuffers_textured", Some(TRANSLUCENT_BLEND))
    );
    // The hand falls back to gbuffers_textured and does not blend; its translucent items
    // fall back to the same program, after deferred and blended.
    let hand = post.hand.as_ref().unwrap();
    assert_eq!(
        (hand.name.as_str(), hand.group, hand.blend),
        ("gbuffers_textured", Group::Gbuffers, None)
    );
    let hand_water = post.hand_water.as_ref().unwrap();
    assert_eq!(
        (hand_water.name.as_str(), hand_water.group, hand_water.blend),
        (
            "gbuffers_textured",
            Group::Translucent,
            Some(TRANSLUCENT_BLEND)
        )
    );
    // Weather falls back to gbuffers_textured, after deferred, blended.
    let weather = post.weather.as_ref().unwrap();
    assert_eq!(
        (weather.name.as_str(), weather.group, weather.blend),
        (
            "gbuffers_textured",
            Group::Translucent,
            Some(TRANSLUCENT_BLEND)
        )
    );
    let ordering = Properties::parse(
        "particles.ordering = before\nseparateEntityDraws = true\n",
        &macros,
    );
    let post = load_post(&pack, "", &ordering, &macros, &inputs);
    assert!(!post.particles_late && post.translucent_late);
    let properties = Properties::parse(
        "blend.gbuffers_terrain = ONE ONE ZERO ONE\nblend.gbuffers_water = off\n",
        &macros,
    );
    let terrain_blend = blend(&properties, "gbuffers_terrain");
    assert_eq!(
        terrain_blend,
        Some(Some([Factor::One, Factor::One, Factor::Zero, Factor::One]))
    );
    assert_eq!(blend(&properties, "gbuffers_water"), Some(None));
    assert_eq!(blend(&properties, "gbuffers_basic"), None);
}

#[test]
fn formats_come_from_the_fragment_shaders_iris_reads() {
    // Cut down from probe packs p9063a, p9063b and p9063c (ours). Iris 1.11.7: colortex8,
    // declared only in `composite.vsh`, kept RGBA8; colortex12 and colortex6, declared only
    // in `shadow.fsh` and `shadowcomp.fsh` of a pack whose shadow pass Sunrise doesn't run,
    // took RGBA16F; colortex1 with `gdepth` declared and no format kept RGBA8. Other
    // settings of those shadow files weren't probed, and Sunrise leaves them out.
    let declare = |n: u32| format!("/*\nconst int colortex{n}Format = RGBA16F;\n*/\n");
    let write = "#version 120\n/* RENDERTARGETS: 1,6,8,12 */\nvoid main() { gl_FragData[0] = \
                 vec4(2.5); gl_FragData[1] = vec4(2.5); gl_FragData[2] = vec4(2.5); \
                 gl_FragData[3] = vec4(2.5); }\n";
    let pack = Pack::from_zip_bytes(
        "original-format-files",
        zip(&[
            (
                "shaders/composite.vsh",
                format!("{}{VSH}", declare(8)).as_str(),
            ),
            ("shaders/composite.fsh", write),
            ("shaders/shadow.vsh", VSH),
            (
                "shaders/shadow.fsh",
                format!(
                    "{}const bool colortex12Clear = false;\n{write}",
                    declare(12)
                )
                .as_str(),
            ),
            ("shaders/shadowcomp.vsh", VSH),
            (
                "shaders/shadowcomp.fsh",
                format!("{}{write}", declare(6)).as_str(),
            ),
            ("shaders/final.vsh", VSH),
            (
                "shaders/final.fsh",
                "#version 120\nuniform sampler2D gdepth;\nvarying vec2 uv;\n\
                 void main() { gl_FragColor = texture2D(gdepth, uv); }\n",
            ),
        ]),
    )
    .unwrap();
    let macros = Macros::standard();
    let post = load_post(
        &pack,
        "",
        &Properties::default(),
        &macros,
        &GameInputs::default(),
    );
    assert!(!post.shadow_pass);
    let half = crate::Format::parse("RGBA16F").unwrap();
    let formats: Vec<bool> = [1, 6, 8, 12]
        .iter()
        .map(|&n| post.config.buffers[n].format == half)
        .collect();
    assert_eq!(formats, [false, true, false, true]);
    assert_eq!(post.config.buffers[1].format, crate::Format::RGBA8);
    assert!(post.config.buffers[12].clear);
}

#[test]
fn sunrises_test_pack_loads() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/post-test");
    let pack = Pack::open(&dir).unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse(&pack.read_text("shaders.properties").unwrap(), &macros);
    let post = load_post(
        &pack,
        &dimension_folder(&pack, 0),
        &properties,
        &macros,
        &GameInputs::default(),
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let names: Vec<&str> = post.programs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["composite", "composite1", "final"]);
    assert_eq!(post.programs[0].buffers, [0, 2]);
    assert_eq!(post.programs[1].buffers, [0]);
    assert_eq!(
        post.config.buffers[2].format,
        crate::Format::parse("RGBA16F").unwrap()
    );
}
