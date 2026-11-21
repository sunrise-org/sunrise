//! A pack's `core/clouds` as the atlas loads it (`load_clouds`).
use super::*;
use sunrise_assets::resource_source::Resource;

const VSH: &str = "assets/minecraft/shaders/core/clouds.vsh";
const FSH: &str = "assets/minecraft/shaders/core/clouds.fsh";

/// `clouds.vsh` in shape: `CloudInfo` and the `CloudFaces` texel buffer.
const CLOUDS_VSH: &str = "#version 330\n\
    layout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    vec3 CloudOffset;\n    vec3 CellSize;\n};\n\
    uniform isamplerBuffer CloudFaces;\nlayout(location = 1) out vec4 vertexColor;\n\
    void main() {\n    int x = texelFetch(CloudFaces, gl_VertexIndex / 4 * 3).r;\n\
        gl_Position = vec4(vec3(x) * CellSize + CloudOffset, 1.0);\n    vertexColor = CloudColor;\n}\n";
const CLOUDS_FSH: &str = "#version 330\nlayout(location = 1) in vec4 vertexColor;\n\
    layout(location = 0) out vec4 fragColor;\nvoid main() {\n    fragColor = vertexColor;\n}\n";

fn source(files: &[(&str, &str)]) -> ResourceSource {
    ResourceSource::indexed(
        files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), Resource::bytes(t.as_bytes().to_vec())))
            .collect(),
    )
}

/// The game's two stages, and a pack's fragment stage over them: the pack's clouds load with
/// `CloudInfo` and `CloudFaces` given; with only the game's stages there is no pack shader; a
/// pack stage reading another block is refused.
#[test]
fn a_pack_replacing_a_clouds_stage_is_loaded_with_the_faces_and_cloud_info() {
    let defaults = source(&[(VSH, CLOUDS_VSH), (FSH, CLOUDS_FSH)]);
    let pink = CLOUDS_FSH.replace(
        "vertexColor;\n}",
        "vertexColor * vec4(1.0, 0.6, 0.8, 1.0);\n}",
    );
    let root = source(&[(VSH, CLOUDS_VSH), (FSH, &pink)]);
    let program = load_clouds(&root, &defaults).expect("the pack's clouds");
    assert_eq!(program.other, ["CloudInfo", "CloudFaces"]);
    assert!(
        load_clouds(&defaults, &defaults).is_none(),
        "the game's own"
    );
    let other = pink.replace(
        "void main()",
        "layout(std140) uniform Other {\n    vec4 Tint;\n};\nvoid main()",
    );
    let other = other.replace("vec4(1.0, 0.6, 0.8, 1.0)", "Tint");
    let root = source(&[(VSH, CLOUDS_VSH), (FSH, &other)]);
    assert!(
        load_clouds(&root, &defaults).is_none(),
        "Other is not given"
    );
}
