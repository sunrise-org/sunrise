//! Files shared by the cloud pack and retained GPU reload fixtures.

/// A `w` by `h` RGBA image with the cells at `filled` opaque.
pub fn png(w: u32, h: u32, filled: &[[u32; 2]]) -> Vec<u8> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for [x, y] in filled {
        rgba[((y * w + x) * 4 + 3) as usize] = 255;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&rgba).unwrap();
    writer.finish().unwrap();
    out
}

/// `resourcepacks/<name>`: a pack with its own `core/sky` (a visible override, which the atlas
/// loads) and a 4 by 6 cloud texture with `cells` filled.
pub fn write_pack(name: &str, cells: &[[u32; 2]]) {
    write_pack_with(name, cells, &[]);
}

/// [`write_pack`] with `own` files besides.
pub fn write_pack_with(name: &str, cells: &[[u32; 2]], own: &[(&str, &str)]) {
    let dir = std::env::current_dir()
        .unwrap()
        .join("resourcepacks")
        .join(name);
    let file = |path: &str, bytes: &[u8]| {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    file(
        "pack.mcmeta",
        br#"{"pack": {"description": "clouds", "pack_format": 97, "min_format": [97, 0], "max_format": [97, 1]}}"#,
    );
    file(
        "assets/minecraft/shaders/core/sky.vsh",
        b"#version 330\nlayout(location = 0) in vec3 Position;\n\
          void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/shaders/core/sky.fsh",
        b"#version 330\nlayout(location = 0) out vec4 fragColor;\n\
          void main() {\n    fragColor = vec4(1.0, 0.0, 0.0, 1.0);\n}\n",
    );
    file(
        "assets/minecraft/textures/environment/clouds.png",
        &png(4, 6, cells),
    );
    for (path, text) in own {
        file(path, text.as_bytes());
    }
}

/// A tall texture vanilla reads inside (`tall_inner` of the Java comparison).
pub const INNER: &[[u32; 2]] = &[[1, 0], [2, 1], [1, 3], [2, 5]];
