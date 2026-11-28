// Helpers of a shader pack's post-processing (pack_post.rs): the world into colortex0 and
// depthtex0 (a depth attachment, for terrain's depth test), and colortex0 onto the screen for
// packs without `final`.

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@group(0) @binding(0) var source: texture_2d<f32>;
// A depth view read as an unfilterable float: naga's GLSL backend cannot
// `textureLoad` a depth texture, and the texel is the same either way.
@group(0) @binding(1) var world_depth: texture_2d<f32>;

struct World {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

// Pack buffers keep GL's rows (row 0 at the bottom), and GL's window depth: Sunrise's
// reverse-Z depth is 1 - d for the same near and far planes.
@fragment
fn fs_blit(@builtin(position) p: vec4<f32>) -> World {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(i32(p.x), size.y - 1 - i32(p.y));
    var out: World;
    out.color = textureLoad(source, at, 0);
    out.depth = 1.0 - textureLoad(world_depth, at, 0).r;
    return out;
}

// The same without depthtex0, for packs that never read it.
@fragment
fn fs_blit_color(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    return textureLoad(source, vec2<i32>(i32(p.x), size.y - 1 - i32(p.y)), 0);
}

// Export the pack's GL-rows, GL-depth `depthtex0` back to Sunrise's screen-rows, reversed-Z
// world depth buffer so post-pack stages (`AfterLevel`, `world_drawn`) depth-test against
// terrain, water, entities, and the first-person hand drawn by the pack.
@fragment
fn fs_export_depth(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    let size = vec2<i32>(textureDimensions(world_depth));
    let at = vec2<i32>(i32(p.x), size.y - 1 - i32(p.y));
    return 1.0 - textureLoad(world_depth, at, 0).r;
}

@group(0) @binding(2) var linear: sampler;

struct Full {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_uv(@builtin(vertex_index) i: u32) -> Full {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    // Screen top (uv.y = 0 in wgpu) shows the buffer's top row, GL's last.
    return Full(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), vec2<f32>(uv.x, 1.0 - uv.y));
}

// The screen's own rows: a picture a shader pack's `final` drew (not a GL-rows buffer).
@vertex
fn vs_screen(@builtin(vertex_index) i: u32) -> Full {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return Full(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

@fragment
fn fs_copy(v: Full) -> @location(0) vec4<f32> {
    return textureSample(source, linear, v.uv);
}

// `fs_copy` for a pack served Sunrise's HDR without a `final`: colortex0 holds linear light, 1.0
// at paper white, which goes onto the frame as a served `final`'s does (the translator's
// `LINEAR_ENCODE`): a negative or NaN channel is 0, then vanilla's 2.2 power, at most half
// float's largest value (docs/hdr-passes.md, rule 5).
fn linear_light(c: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), c, c >= vec3<f32>(0.0));
}

fn encoded(light: vec3<f32>) -> vec3<f32> {
    return min(pow(light, vec3<f32>(1.0 / 2.2)), vec3<f32>(65504.0));
}

// With a render scale: it filters the light as `fs_copy` does (bilinear, clamped at the edges),
// but itself, from the texels that count: a sampler weighs every texel of the four, and an
// infinity weighed 0 is NaN on some drivers. Alpha is the centre texel's, unfiltered
// (docs/hdr-passes.md, rule 6).
@fragment
fn fs_copy_linear(v: Full) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let at = v.uv * vec2<f32>(size) - 0.5;
    let first = vec2<i32>(floor(at));
    let f = at - floor(at);
    var light = vec3<f32>(0.0);
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let w = select(1.0 - f.x, f.x, x == 1) * select(1.0 - f.y, f.y, y == 1);
            if w > 0.0 {
                let texel = clamp(first + vec2<i32>(x, y), vec2<i32>(0), size - 1);
                light += w * linear_light(textureLoad(source, texel, 0).rgb);
            }
        }
    }
    let centre = clamp(vec2<i32>(floor(v.uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
    return vec4<f32>(encoded(light), textureLoad(source, centre, 0).a);
}

// Without a render scale: each pixel is its own texel, read by its integer position (GL's rows,
// as `vs_uv` flips them), so no neighbour's light reaches it through a filter weight that the
// interpolated coordinate leaves a little above 0.
@fragment
fn fs_copy_linear_texel(v: Full) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let p = vec2<i32>(v.position.xy);
    let c = textureLoad(source, vec2<i32>(p.x, size.y - 1 - p.y), 0);
    return vec4<f32>(encoded(linear_light(c.rgb)), c.a);
}

// The upscale of a served `final`'s picture (already encoded, finite and not negative): its
// colour filtered as `fs_copy` filters it, its alpha the centre texel's, unfiltered
// (docs/hdr-passes.md, rule 6).
@fragment
fn fs_upscale_linear(v: Full) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let centre = clamp(vec2<i32>(floor(v.uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
    let rgb = textureSample(source, linear, v.uv).rgb;
    return vec4<f32>(rgb, textureLoad(source, centre, 0).a);
}

// One level of a buffer's mip chain from the level above (6.1.7): a box filter, the average
// of the texels under each destination texel (OpenGL 4.6, 8.14.4, recommends one). Along an
// even side a texel covers two of the level above; along an odd side of 2d + 1 texels, texel i
// covers 2 + 1 / d of them: 2i, 2i + 1 and 2i + 2, weighted d - i, d and i + 1 over 2d + 1.
// Every texel of the level above then counts as much, so a small bright spot keeps its share
// down to the last level, which is the buffer's mean.
fn down_weights(i: u32, side: u32) -> vec3<f32> {
    if side == 1u {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    if side % 2u == 0u {
        return vec3<f32>(0.5, 0.5, 0.0);
    }
    let d = f32(side / 2u);
    return vec3<f32>(d - f32(i), d, f32(i) + 1.0) / f32(side);
}

@fragment
fn fs_down(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let above = textureDimensions(source);
    let at = vec2<u32>(p.xy);
    let wx = down_weights(at.x, above.x);
    let wy = down_weights(at.y, above.y);
    let first = vec2<i32>(at * 2u);
    let last = vec2<i32>(above) - 1;
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < 3; y++) {
        for (var x = 0; x < 3; x++) {
            let w = wx[x] * wy[y];
            if w > 0.0 {
                sum += w * textureLoad(source, min(first + vec2<i32>(x, y), last), 0);
            }
        }
    }
    return sum;
}
