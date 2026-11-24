//! Client render pipeline registration.

pub trait Hooks {
    /// Register render pipelines once at client startup (NeoForge's
    /// `RegisterRenderPipelinesEvent`; 1.21.1 mods used `RegisterShadersEvent`): call
    /// `event.register(pipeline)` with each [`RenderPipeline`]. The game builds them with its own
    /// at startup and again on every resource reload. Any valid [`Identifier`] is accepted, and
    /// duplicate mod IDs fail startup, regardless of required or optional policy. Collisions
    /// with vanilla are checked in the corresponding required or optional table. An unavailable
    /// vanilla pipeline table fails startup registration. A required pipeline whose
    /// shader is missing or doesn't build fails the load: the game does not start, and a reload
    /// fails and keeps the previous resources. An [optional](RenderPipeline::optional()) one is
    /// logged and resolves to [`RenderPipelineError::Invalid`].
    #[notes = r#"## Render pipelines

`Hooks::register_render_pipelines` is the client registration-phase counterpart of NeoForge's
`RegisterRenderPipelinesEvent` (and of 1.21.1's `RegisterShadersEvent`, which it replaced). A
pipeline is plain data: its id, a WGSL fragment shader in the mod's assets, a blend mode and a
depth mode. Pass it to `event.register(pipeline)`; registration takes no GPU object or shader handle.

```rust,ignore
fn register_render_pipelines(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterRenderPipelinesEvent<'_>,
) {
    event.register(RenderPipeline::new(
        Identifier::parse("example:glow").unwrap(),
        Identifier::parse("example:glow").unwrap(),
        PipelineBlend::Additive,
        WorldDepth::Tested,
    ));
}
```

The shader `example:glow` is `assets/example/shaders/pipeline/glow.wgsl`. It defines one function;
the game supplies the vertex stage, the bindings and the conversion to the frame:

```wgsl
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    let pulse = 1.0 + sin(frame.time * 4.0);
    return vec4(input.color.rgb * pulse, input.color.a * (1.0 - input.uv.y));
}
```

`PipelineInput` has `uv` (the quad's texture coordinate), `color` (the quad's colour) and
`position` (blocks, relative to the camera). `frame.time` is seconds, wrapping every hour, and
`frame.viewport` is the world picture's size in pixels. Colours follow the
mod render colour rule: the function reads and returns linear
light, 1.0 is paper white, more is brighter, and alpha is coverage. A file that declares its own
bindings or entry points is refused, and so is one
that SPIR-V, MSL or HLSL translation refuses (the refusal names the backend), or one over 131,072
bytes or with an expression chaining more than 200 binary operators, member accesses or indexings
(`a + b + c + ...`: split it with `let`), which would overflow the parser's stack. A refusal with a
place in the file reads `<file>:<line>:<column>: <reason>`.

Resolve the pipeline with `host.render_assets().render_pipeline(&id)` in `client_setup`
and again in `client_resources_reloaded`, then draw with `WorldDrawContext::pipeline_quads` from `submit_custom_geometry` (or a debug renderer).
A resource reload builds every registered pipeline again, so a resource pack can replace a shader;
a handle from before the reload is `Stale`.

A pipeline is required unless it is registered with `.optional()` (NeoForge's
`registerPipeline` and `registerOptionalPipeline`). Any valid `Identifier` is accepted, and
a mod pipeline ID must be unique across both policies, because mod lookups are by ID. A
collision with vanilla is checked in the corresponding required or optional table. When a load
cannot build a required pipeline because its file is missing, refused or turned down by the GPU, the whole load fails and names it:
the game does not start, and a reload keeps the previous resources and handles, then drops the
selected resource packs and tries again, as vanilla does with any failed reload. An optional
pipeline that cannot be built is logged once with its id and resolves to `Invalid`, and the other
pipelines draw. Pipeline quads grow dynamically into the GPU buffers with no fixed per-view cap
(skipping any quad with non-finite coordinates or UVs individually); in the deferred world pass,
pipeline quads draw after the plain quads and lines, in submission order."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "pipelines",
        registration_append = "attributed",
        context = "registration",
        neoforge = "RegisterRenderPipelinesEvent",
        version = 4
    )]
    fn register_render_pipelines(pipelines: &mut Vec<mod_api::RenderPipeline>);
}
