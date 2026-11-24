# Drawing with your own render pipeline

Draw world quads with your own fragment shader. Register a `RenderPipeline`, resolve
its handle, and submit quads through a `WorldDrawContext` (in `submit_custom_geometry` or
a debug renderer). This is the restricted counterpart of NeoForge's
`RegisterRenderPipelinesEvent`: you supply the fragment stage; the game fixes the vertex
stage, primitive type and bindings.

## 1. Write the shader

Put a WGSL file at `assets/<namespace>/shaders/pipeline/<path>.wgsl`. It defines one function:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let pulse = 1.0 + sin(frame.time * 4.0);
    return vec4(input.color.rgb * pulse, input.color.a * (1.0 - input.uv.y));
}
```

The game puts this before your file:

| Name | Type | Meaning |
| --- | --- | --- |
| `input.uv` | `vec2<f32>` | The quad's texture coordinate at this fragment, as its corners gave it. |
| `input.color` | `vec4<f32>` | The quad's colour in linear light; alpha is coverage. |
| `input.position` | `vec3<f32>` | This point in blocks, relative to the camera. |
| `frame.time` | `f32` | Seconds of game time, smooth between ticks, wrapping to 0 every hour. |
| `frame.viewport` | `vec2<f32>` | The world picture's size in pixels. |

The function reads and returns linear light: 1.0 is paper white, and more is brighter where the frame can hold it. The
alpha you return is coverage. The game supplies the vertex stage, the bindings and the conversion
to the frame, so a file that declares a binding or an entry point is refused. Names that start with
`sunrise_` belong to the game.

The game parses the file before it can compile it, and a very deep expression would overflow the
parser's stack, so a file longer than 131,072 bytes (the whole shader, with the game's own code
around yours), or an expression that chains more than 200 binary operators, member accesses or
indexings (`a + b + c + ...`), is refused by name: split a long sum over several `let` statements.

## 2. Register the pipeline

Call `event.register(pipeline)` in `register_render_pipelines` at client startup.
The event adds declarations in order and does not expose earlier registrations.
Resource reloads rebuild the registered pipelines from the current shader files;
they do not call the registration hook again.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(RenderPipeline::new(
        Identifier::parse("example:glow").unwrap(), // the pipeline's id
        Identifier::parse("example:glow").unwrap(), // assets/example/shaders/pipeline/glow.wgsl
        PipelineBlend::Additive,
        WorldDepth::Tested,
    ));
}
```

`PipelineBlend::Translucent` lays the result over the frame by its alpha, and `Additive` adds it,
scaled by its alpha. `WorldDepth::Tested` hides the quads behind nearer world surfaces, and
`ThroughWalls` draws them over everything. No pipeline writes depth.

Any valid `Identifier` is accepted. Use a unique ID for every mod pipeline, regardless of its
required or optional policy. A mod ID collision fails startup. Collisions with vanilla are checked
in the corresponding required or optional table. If startup reports missing or invalid
`render-pipelines.json`, prepare the client resources again.

### Required or optional

A pipeline is required unless you register it with `.optional()`, as NeoForge's `registerPipeline`
and `registerOptionalPipeline` are:

```rust,ignore
event.register(
    RenderPipeline::new(id, fragment, PipelineBlend::Translucent, WorldDepth::Tested).optional(),
);
```

| Policy | When a resource load cannot build it |
| --- | --- |
| Required (`RenderPipeline::new`) | The load fails with a message naming each required pipeline and why. Startup cannot finish with a broken required pipeline. A failed resource reload keeps the previous resources, so handles from before it still draw. |
| Optional (`.optional()`) | The log has one line naming the pipeline and why. It resolves to `RenderPipelineError::Invalid`, and every other pipeline builds and draws. |

Client resource pack recovery only runs if an optional pack was selected. It clears and saves the
failing selection before reloading the required packs and mod content. If recovery fails, the
error is reported and that selection stays cleared. If the player has already committed a newer
selection during startup, the queued load uses it instead. Create World's data-pack editor is
unaffected.

A load cannot build a pipeline when its shader file is missing, the file is refused, or the GPU
rejects the shader. The game never panics over a pipeline's shader.

### Checking a shader before you run the game

`cargo sunrise-mod check`, run in the mod's folder, composes and validates every
`assets/<namespace>/shaders/pipeline/**/*.wgsl` file with the code the game uses, and fails with
`<file>:<line>:<column>: <reason>` for each one the game would refuse (the same text the game logs
after the pipeline's id; a refusal with no place in the file, such as a missing function, is
`<file>: <reason>`, and a place in the game's own prelude reads `<prelude>:<line>:<column>`). The
tool cannot see your `RenderPipeline` registration, so it takes a file whose text contains
`pipeline_texture` anywhere (a comment included) as textured and any other file as untextured; the
game goes by the registration's `.textured(...)` instead. A file the check accepts can still be
refused at load: when its registration declares no texture but the file samples one (or the
reverse), when the registration's texture or another resource it names is missing, or when the GPU
rejects the shader.

The check, and the load, also refuse a shader that a player's system could not translate. The
shader is translated to SPIR-V (Vulkan), MSL (Metal) and HLSL (DirectX 12) as the GPU library does,
and a shader that any of them refuses is refused naming it, as in `Vulkan (SPIR-V) refuses it: ...`.
Some shaders pass validation and still fail here, such as passing a pointer to a matrix column or
to an element of a local array to a function; copy the value into a function argument instead.
There is no count of expressions or texture reads: what the GPU library translates is drawn.

## 3. Resolve and draw

```rust,ignore
fn client_setup(&self, context: &mut dyn ClientSetupContext, _event: &mut ClientSetupEvent<'_>) {
    let id = Identifier::parse("example:glow").unwrap();
    *self.glow.lock().unwrap() = context
        .render_assets()
        .render_pipeline(&id)
        .ok();
}

fn submit_custom_geometry(&self, _host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
    let Some(glow) = *self.glow.lock().unwrap() else { return };
    let quad = PipelineQuad::spanning(corners, 0xFFFF_C040);
    if let Err(error) = event.draw.pipeline_quads(glow, &[quad]) {
        log::warn!("glow not drawn: {error:?}");
    }
}
```

Resolve again in `client_resources_reloaded`: every resource reload builds the pipelines again
from the new resources, so a resource pack can replace your shader, and a handle from before the
reload answers `Stale`.

| Error | When |
| --- | --- |
| `RenderPipelineError::Unknown` | No mod registered that id. |
| `RenderPipelineError::Invalid` | The pipeline is optional and its shader file is missing, was refused or was rejected by the GPU. The log has a line `render pipeline <id> from mod <owner> refused: <file>: <reason>`, with the file's line number for a syntax or type error, or `render pipeline <id> from mod <owner> refused by the GPU: <reason>`. |
| `RenderPipelineError::Stale` | The handle is from before a resource reload. |

`pipeline_quads` returns `PipelineDrawError::Pipeline(...)` when the handle is invalid or stale, drawing nothing for that call. Any quad with a non-finite corner, camera-relative `f32` overflow, or non-finite texture coordinate is ignored while the valid quads in the batch still draw, and quad storage grows dynamically into the GPU buffers without a 4,096 cap. An `Invalid` pipeline never stops the game from starting; a required pipeline that cannot be built does.

## What a pipeline draws, and when

`WorldDrawContext::pipeline_quads` in `submit_custom_geometry` or a debug renderer submits quads
for the deferred world pass. Quads with non-finite coordinates or UVs are skipped individually
without rejecting valid quads in the same slice; accepted pipeline quads draw after the pass's
plain quads and lines, in submission order.

## Sampling a texture

A pipeline can sample one texture (vanilla's `Sampler0`). Name it when you register the pipeline:

```rust,ignore
event.register(
    RenderPipeline::new(id.clone(), id, PipelineBlend::Additive, WorldDepth::Tested)
        .textured(PipelineTexture::Sprite(Identifier::parse("example:block/charge").unwrap())),
);
```

`PipelineTexture::Sprite(id)` names a sprite of the block or item atlas by its texture id:
`example:block/charge` is `assets/example/textures/block/charge.png` as the `minecraft:blocks`
atlas holds it, and `minecraft:item/diamond` names an item sprite. Animated sprites keep their
animation. A textured pipeline samples a rectangle of the atlas; it does not sample a standalone
texture file.

The shader then has one more function:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    // The texture in linear light, tinted by the quad's colour, as vanilla's position_tex_color does.
    return pipeline_texture(input.uv) * input.color;
}
```

`pipeline_texture(uv)` maps the fraction of `uv` into the sprite's own rectangle (`uv` 0 to 1
spans it once; beyond that, negative values too, it repeats) and loads one texel by integer
coordinates at mip level 0: nearest, unfiltered, so no texel of a neighbouring sprite can enter,
even at the far edge. It returns the texel in linear light with its alpha. `input.uv` is still
what the quad's corners gave, so the quads you already draw keep their coordinates.

A texture is part of the pipeline, so it is checked when the pipeline is built, at startup and on
each reload. A refused texture is named by `PipelineTextureError` and refuses the pipeline as a
missing shader does: a required one fails the load, and an optional one is logged with the named
cause and resolves to `Invalid`:

| Cause | When |
| --- | --- |
| `NoSuchSprite` | Neither the block nor the item atlas has the sprite (`… refused by the GPU: its texture sprite <name>: NoSuchSprite: …`) |
| `NotASpriteKey` | The id names a file (a `textures/` path or a `.png` name), not a sprite key |

A pipeline without `.textured(…)` has no `pipeline_texture`: its file is refused with a note
naming `RenderPipeline::textured`.

A pipeline cannot sample more than one texture or replace the vertex stage; custom per-frame values
are available for translated shader-pack uniforms via `WorldDrawContext::set_pack_uniform` (below).

## Participating in translated shader packs

To let an active Iris/OptiFine shader pack shade a pipeline's geometry with its own program, declare
its `PipelineGeometryClass` on registration via `.geometry_class(...)`:

```rust,ignore
event.register(
    RenderPipeline::new(id.clone(), id, PipelineBlend::Translucent, WorldDepth::Tested)
        .geometry_class(PipelineGeometryClass::Terrain),
);
```

Each geometry class selects a standard Iris/OptiFine program fallback chain (for example,
`Terrain` resolves `gbuffers_terrain -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`,
and `Particles` resolves `gbuffers_particles -> gbuffers_textured_lit -> gbuffers_textured -> gbuffers_basic`).
Pipelines that need pack-defined programs, preprocessor defines, an auxiliary depth target, or
per-frame pack uniforms attach an optional `.pack_binding(PackBinding::new()...)`. Startup refuses
invalid GLSL identifiers, reserved Iris/OptiFine sampler/image/uniform names, and cross-mod or
namespace collisions across samplers, uniforms, defines, and depth-target IDs.

During a world draw hook, feed declared per-frame pack uniforms with
`ctx.set_pack_uniform(name, PackUniformValue::...)` (returns `false` without panicking when `name`
or its `PackUniformType` is not registered; any `previous_name` declared on the uniform is tracked
automatically across frames). To inspect whether an active shader pack bound a specific pipeline to
one of its translated programs (`PackOverrideState::Bound`), left it on the mod's own fallback
shader (`PackOverrideState::UnboundPack`), or no pack is active (`PackOverrideState::NoPack`), call
`ctx.pack_override_state(pipeline)`. To check generally whether a shader pack is currently loaded
and rendering the world (always `false` on the title screen, matching `IrisApi.isShaderPackInUse()`),
call `host.shader_pack_active()`.
