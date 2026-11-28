//! Original terrain-fade controls, authored from the allowed pack API names only.
use sunrise_shaders::{
    GbuffersInputs, GeometryInputs, Macros, Preprocessed, Properties, Target, translate_composite,
    translate_gbuffers, translate_geometry,
};

#[test]
fn requested_fade_feature_loads() {
    let properties = Properties::parse(
        "iris.features.optional=FADE_VARIABLE\niris.features.required=FADE_VARIABLE\n",
        &Macros::standard(),
    );
    let notes = Macros::standard().feature_notes(&properties);
    assert!(notes.is_empty(), "requested fade rejected: {notes:?}");
}

#[test]
fn terrain_vertex_reads_renderer_fade_without_a_pack_declaration() {
    let pp = |code: &str| Preprocessed::from_source("", code);
    let vertex =
        pp("varying float seen;\nvoid main() { seen = mc_chunkFade; gl_Position = gl_Vertex; }");
    let fragment =
        pp("varying float seen;\nvoid main() { gl_FragColor = vec4(seen, 0.0, 0.0, 1.0); }");
    let inputs = GbuffersInputs {
        attributes: vec![("mc_chunkFade".into(), "0.375".into())],
        ..Default::default()
    };
    let result = translate_gbuffers(
        &vertex,
        &fragment,
        Target::Buffers,
        &inputs,
        naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
    );
    assert!(
        result.is_ok(),
        "terrain fade translation: {:?}",
        result.err()
    );
}

#[test]
fn missing_terrain_value_is_an_error_instead_of_an_invented_fade() {
    let vertex = Preprocessed::from_source("", "void main() { gl_Position = vec4(mc_chunkFade); }");
    let fragment = Preprocessed::from_source("", "void main() { gl_FragColor = vec4(1.0); }");
    assert!(
        translate_gbuffers(
            &vertex,
            &fragment,
            Target::Buffers,
            &GbuffersInputs::default(),
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT
        )
        .is_err()
    );
    assert!(
        translate_composite(
            &vertex,
            &fragment,
            Target::Buffers,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT
        )
        .is_err()
    );
    let supplied = GbuffersInputs {
        attributes: vec![("mc_chunkFade".into(), "0.25".into())],
        ..Default::default()
    };
    let fragment =
        Preprocessed::from_source("", "void main() { gl_FragColor = vec4(mc_chunkFade); }");
    let vertex = Preprocessed::from_source("", "void main() { gl_Position = gl_Vertex; }");
    assert!(
        translate_gbuffers(
            &vertex,
            &fragment,
            Target::Buffers,
            &supplied,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT
        )
        .is_err()
    );
}

#[test]
fn authored_fade_globals_keep_their_own_meaning() {
    let inputs = GbuffersInputs {
        attributes: vec![("mc_chunkFade".into(), "0.25".into())],
        ..Default::default()
    };
    let fragment = Preprocessed::from_source("", "void main() { gl_FragColor = vec4(1.0); }");
    for declaration in [
        "const float mc_chunkFade = 0.75;",
        "uniform float mc_chunkFade;",
    ] {
        let vertex = Preprocessed::from_source(
            "",
            &format!("{declaration}\nvoid main() {{ gl_Position = vec4(mc_chunkFade); }}"),
        );
        let program = translate_gbuffers(
            &vertex,
            &fragment,
            Target::Buffers,
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
        .unwrap();
        assert_eq!(
            program
                .interface
                .uniforms
                .iter()
                .any(|u| u.name == "mc_chunkFade"),
            declaration.starts_with("uniform")
        );
    }
}

#[test]
fn fade_feature_negotiation_ignores_case_and_defines_the_written_macro() {
    // The request matches in any case; the answer is `IRIS_FEATURE_FADE_VARIABLE` as Sunrise
    // writes the flag (decision D-7), never the pack's own spelling.
    for (request, rejected, probe) in [
        ("", false, "opaque"),
        (
            "iris.features.required=FADE_VARIABLE\n",
            false,
            "transition",
        ),
        (
            "iris.features.required=fade_variable\n",
            false,
            "transition",
        ),
        (
            "iris.features.optional=fade_variable\n",
            false,
            "transition",
        ),
        ("iris.features.required=UNKNOWN_FADE\n", true, "opaque"),
    ] {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .start_file(
                "shaders/shaders.properties",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let text = format!(
            "{request}#ifdef IRIS_FEATURE_FADE_VARIABLE\nprobe=transition\n#else\nprobe=opaque\n#endif\n\
             #ifdef IRIS_FEATURE_fade_variable\nspelled=pack\n#endif\n"
        );
        std::io::Write::write_all(&mut archive, text.as_bytes()).unwrap();
        let pack = sunrise_shaders::Pack::from_zip_bytes(
            "fade.zip",
            archive.finish().unwrap().into_inner(),
        )
        .unwrap();
        let (properties, notes) = sunrise_shaders::read_properties(&pack, &mut Macros::standard());
        assert_eq!(!notes.is_empty(), rejected, "{request}");
        assert_eq!(properties.get("probe"), Some(probe), "{request}");
        assert_eq!(properties.get("spelled"), None, "{request}");
    }
}

fn geometry_probe(vertex_declaration: &str) -> [Preprocessed; 3] {
    [
        format!("{vertex_declaration}\nout float terrain_weight; void main() {{ terrain_weight = mc_chunkFade; gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }}"),
        "layout(triangles) in; layout(triangle_strip, max_vertices=3) out; in float terrain_weight[]; out float seen; const float mc_chunkFade = 0.8125; void main() { for (int k=0; k<3; ++k) { seen=terrain_weight[k] + mc_chunkFade; gl_Position=gl_in[k].gl_Position; EmitVertex(); } EndPrimitive(); }".into(),
        "in float seen; void main() { gl_FragColor = vec4(seen); }".into(),
    ].map(|code| Preprocessed::from_source("", &code))
}

#[test]
fn geometry_vertex_reads_the_provided_fade_and_requires_its_provider() {
    let [vertex, mut geometry, fragment] = geometry_probe("");
    let mut inputs = GeometryInputs {
        inputs: GbuffersInputs {
            body: "float hz_probe_transition = 0.375;".into(),
            attributes: vec![("mc_chunkFade".into(), "hz_probe_transition".into())],
            ..Default::default()
        },
        index: "@".into(),
        ..Default::default()
    };
    let (_, count) = translate_geometry(
        &vertex,
        &geometry,
        &fragment,
        &inputs,
        naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
    )
    .unwrap();
    assert_eq!(count, 1);
    inputs.inputs.attributes.clear();
    geometry =
        Preprocessed::from_source("", &geometry.code().replace("mc_chunkFade", "stage_bias"));
    assert!(
        translate_geometry(
            &vertex,
            &geometry,
            &fragment,
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT
        )
        .is_err()
    );
}

#[test]
fn geometry_vertex_keeps_authored_fade_globals() {
    let inputs = GeometryInputs {
        inputs: GbuffersInputs {
            attributes: vec![("mc_chunkFade".into(), "0.25".into())],
            ..Default::default()
        },
        index: "@".into(),
        ..Default::default()
    };
    for declaration in [
        "const float mc_chunkFade = 0.75;",
        "uniform float mc_chunkFade;",
    ] {
        let [vertex, mut geometry, fragment] = geometry_probe(declaration);
        geometry =
            Preprocessed::from_source("", &geometry.code().replace("mc_chunkFade", "stage_bias"));
        let (program, _) = translate_geometry(
            &vertex,
            &geometry,
            &fragment,
            &inputs,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
        .unwrap();
        assert_eq!(
            program
                .interface
                .uniforms
                .iter()
                .any(|u| u.name == "mc_chunkFade"),
            declaration.starts_with("uniform")
        );
    }
}
