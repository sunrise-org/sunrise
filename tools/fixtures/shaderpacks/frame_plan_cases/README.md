# Frame plan: cases

A test fixture of `tools/pack_plan_trace.py`, not a look and not offered to players: a shader pack in the `shaders/`
format (GLSL 120, [shader-packs-spec.md](../../../../docs/cleanroom/shader-packs-spec.md)) whose frames reach the
cases the frame planner's measure must see (#6552):

- a shadow program, no image or storage buffer, and no entity or block entity in the shadow pass
  (`shadowEntities=false`, `shadowBlockEntities=false`), so with the Shadow Cache on and a frozen world a frame
  reuses the shadow map (a cache hit);
- `deferred` writes colortex1 (`size.buffer` 427×240 pixels) and colortex2 (half the render size): at 854×480 the two
  sizes meet and it runs; at half that render size they differ and it is skipped, while `deferred1`, which writes
  colortex2 alone, runs;
- `composite` reads them, the shadow map and colortex0; `final` shows colortex3.
