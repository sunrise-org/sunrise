# Shader packs

Sunrise runs shader packs in the Iris/OptiFine `shaders/` format: a folder or a `.zip` in `shaderpacks/` with a
`shaders/` folder inside, written in GLSL. The format Sunrise implements is defined in
[cleanroom/shader-packs-spec.md](cleanroom/shader-packs-spec.md); `crates/sunrise-shaders` reads a pack and translates
its GLSL, and the renderer draws it.

Sunrise's own WGSL pack format (`sunrise-shaders.toml` and the `.sshader` package, API 0.1 and 0.2) is removed. A
`.sshader` file or such a folder in `shaderpacks/` is left out of the list with a note that its format is no longer
supported, and a saved choice of one falls back to no pack with the same note. The registry lists no such pack.

## Trying a pack in the game

Put the pack folder (or its `.zip`) in `shaderpacks/` beside `options.txt` (Open Pack Folder opens it) and pick
it on Options > Sunrise Settings > Shaders; one runs at a time, and clicking it again turns it off. Its
Shader Options... row opens the picked pack's own settings.

The Shaders page of Sunrise Settings has a Shader pack key for the same pick: press it and the picked pack turns off,
press it again and the same pack runs again. It is bound there (click the row, then press a key; Escape unbinds it),
`K` out of the box, and it is not a vanilla key binding, so rebinding it never turns into another mapping's key.
Turning off remembers the pick in the Extras settings,
so the pack stays off across restarts until the key is pressed again; the menu's second click on the picked pack
does the same.

A picked pack's options open from Shaders' **Shader Options...** row, on a screen of their own, under the pack's name
in its page list: its screens, with the names the pack gives them, its profiles, and Reset. The row is greyed, saying
why, while no pack is picked or the pick's options couldn't be read. The main screen and those it links to are in the list (when
there are more than it has room for, they collapse into one `‹ Name ›` row that the arrows and a controller's
bumpers step through); deeper screens open from their links, and Escape goes back. A click, Enter or Right picks an
option's next value, a Shift click or Left the one before, and the options the pack lists in
`sliders` are sliders. Pointing at one shows what the pack says of it (`option.NAME.comment`); the search finds an
option on any screen. Shader Options' Undo puts back the pack's options as the screen opened with them (and the
remembered pick, which the key can change meanwhile); the pick and the Shadow Cache are Sunrise Settings', whose Undo
puts them back. Picking away from a pack saves its changed options, as Apply does. A pack's
options are saved to `shaderpacks/<pack>.txt` (`NAME=value` lines), and the pack loads again with them, with Apply
(shown while they differ from the saved ones) or on leaving the screen.

Under the picked pack's row, **Iris compatibility** (Auto, On, Off) decides whether the pack gets `IS_IRIS` and
`IRIS_VERSION`, the macros packs test to take the path they wrote for Iris. Auto, the default, gives them only to a
pack whose programs test them (seen while reading its programs with its saved options and feature answers); On and Off
decide for that pack whatever it tests. The choice is kept by the pack's name in `sunrise-graphics.txt`
(`shaderPackIrisCompat:<on|off>:<name>`), apart from the pack's own options, so a pack replaced under the same name
keeps it. While it applies, the pack's row says so and the log has a `runs with Iris compatibility` line; a load
started before the switch changed is dropped and the pack loads again. The rule is decision D-8 in
[cleanroom/decisions.md](cleanroom/decisions.md).

The **Shadow Cache** (off by default; `shadowCache` in `sunrise-graphics.txt`) skips the shadow pass and keeps the
shadow map and shadowcolor buffers of the frame before while nothing that draws them has changed (the world, the
time, the snapped shadow camera, the shadow settings, and the uniforms the shadow programs and their compute stages
read). It keeps them only while the pass would draw the same again: the shadow pass still runs every frame while an
entity casts a shadow, while the pack has `image.*` images, while a shadow program draws into, flips or stores into a
shadowcolor buffer that isn't cleared every frame (`shadowcolorNClear=false`: the program builds on the frame
before), while a shadow program or a compute stage attached to one binds the pack's `bufferObject.*` storage buffers
or reads its work-group counts from one (`indirect.<pass>`) (they keep their contents between frames, so it may read or write something new each frame), and while a program
outside the shadow pass binds `shadowcolorimg0` or `shadowcolorimg1`, since its stores would otherwise pile up in the
kept buffers.

## Testing a pack frame by frame

A pack that keeps something from frame to frame (buffers, images, smoothed values) draws a little differently in every run,
as frames come when they come. With `SUNRISE_STILL=1` and the world frozen the scene holds still
([agent-control.md](agent-control.md#repeatable-screenshots)), and

```sh
sunrise-ctl screenshot-sequence 64 50 frame64.png
```

starts the pack over as if just loaded and draws 64 frames 50 ms apart on its clock: frame n is at n frames and
(n - 1) × 0.05 seconds. The
64th is written to `frame64.png` (and `frame64-hdr.png` with HDR output on) before the reply. STEP_MS may be a
decimal: `16.666667` steps a 60 fps clock. The same build, GPU
and driver write the same bytes every run, so two versions of a pack compare by a pixel diff. `screenshot-sequence
cancel` stops one.

## Sunrise extensions: HDR

A pack can ask Sunrise to show its picture in HDR: light above paper white, up to the display's peak. It's an
addition to the format, under Sunrise's own names, and a pack that doesn't ask runs exactly as without it. For now
it's served only under `SUNRISE_HDR_FRAME=float` (the switch for the float frame, [hdr-passes.md](hdr-passes.md)),
an experimental API. `tools/fixtures/shaderpacks/hdr_extension` is a whole pack that uses it; the examples below are
taken from it.

**Asking.** Two keys in `shaders.properties`:

```properties
sunrise.hdr = true
#ifdef SUNRISE_CURVE
sunrise.hdr.tonemap = sunrise
#else
sunrise.hdr.tonemap = pack
#endif
```

- `sunrise.hdr` is `true` or `false`; `sunrise.hdr.tonemap` is `pack` (without the key too) or `sunrise`. Any other
  value turns the request off, with a note in the log naming the key and the line. Nothing may follow the value on
  its line: a comment goes on a line of its own.
- Sunrise picks the frame before it compiles the pack, so it reads the keys then, with the standard macros and the
  pack's options as chosen: they may sit behind one of the pack's options, as here (`SUNRISE_CURVE` is a switch in
  `final.fsh`). The renderer's macros don't count yet; where the loader's later read with them differs, the
  frame's answer counts and the log says so.

**Served.** The pack is served the extension when the switch is on, the key asked for it when the frame was picked,
and the renderer was made for that request. Whether HDR output is on doesn't change it: on an SDR display the pack
is still served, with SDR's display values, so turning HDR on or off never changes which path the pack compiles.
Served, its programs see these macros, which no other loader defines; anywhere else the pack takes its other path:

| Macro | Value | Means |
| --- | --- | --- |
| `SUNRISE_HDR` | `1`, the extension's version | `final` writes linear light, and the display uniforms are served |
| `SUNRISE_HDR_TONEMAP_PACK` | defined, empty | `sunrise.hdr.tonemap = pack`: the pack owns the tone curve |

**What `final` writes.** Served, output 0 of `final` is linear light: BT.709 primaries, 1.0 at paper white, more is
brighter, any value. Sunrise stores it in the frame's encoding (vanilla's 2.2 power) after the pack's `main`: a
negative or NaN channel becomes 0, and the encoded value, after the 2.2 power, is held at half float's largest,
65504; alpha is stored as written. A pack with no `final` is served the
same way: what it leaves in `colortex0` is read as linear light. Render scale doesn't change any of it.

```glsl
#ifdef SUNRISE_HDR
#ifdef SUNRISE_HDR_TONEMAP_PACK
    // The pack owns the tone curve: what it writes is what the display shows, up to its peak.
    linearColor = min(linearColor, vec3(sunriseHeadroom));
#endif
    // Linear light, 1.0 at paper white: Sunrise stores it in the frame's encoding.
    gl_FragData[0] = vec4(linearColor, 1.0);
#else
    // Anywhere else: the SDR picture, clipped at white and in the 2.2 power, as before.
    gl_FragData[0] = vec4(pow(min(linearColor, vec3(1.0)), vec3(1.0 / 2.2)), 1.0);
#endif
```

Light above white shows only on the float frame, which Sunrise uses with HDR output on where the device can keep the
HUD on a layer of its own. On any other frame (HDR off, or a device without that layer) the picture is clipped at
white, as an SDR pack's is.

**The tone curve has one owner:**

| `sunrise.hdr.tonemap` | `final` writes | Sunrise then |
| --- | --- | --- |
| `pack` | what the display shows, from 0 to `sunriseHeadroom` | runs no tone curve; light past the peak is held at it |
| `sunrise` | the scene's light, any value from 0 | on the float frame, eases the light into the display's peak with its own tone curve, as it does without a pack with HDR Lighting off |

**Display uniforms.** Served only with `SUNRISE_HDR`, every frame (an Auto paper white follows what the display
reports):

| Uniform | Type | SDR, or a frame that isn't the float one | HDR |
| --- | --- | --- | --- |
| `sunriseHdr` | `int` | 0 | 1 |
| `sunriseHeadroom` | `float` | 1.0 | the peak over paper white, at least 1 |
| `sunrisePaperWhite` | `float` | 80 | paper white in nits, as set or as the display reports it (Auto) |
| `sunrisePeak` | `float` | 80 | paper white times the headroom, in nits |
| `sunriseBlackLevel` | `float` | 0 | the black level in nits, at most paper white |

The black level is for information: Sunrise lifts the shadows to it after the tone curve, so the pack doesn't lift
them itself. Served, these names are Sunrise's, ahead of a custom uniform (`uniform.*`) of the same name. Not served,
a program that declares one reads 0, or the pack's custom uniform of that name.

**Everything else is as without the extension.** What a program reads (`fogColor`, `skyColor`, textures, the
colortex buffers) keeps its meaning; a pack that blends linear light does so in its own buffers, in a format it
picks, as on any loader. The hand is part of the pack's picture, as it draws it. The camera overlays, the HUD and
the menus draw over `final`'s picture as over Sunrise's own HDR picture. Of an HDR screenshot, the `-hdr.png` keeps
the pack's light, and the SDR screenshot clips it at white. HDR Lighting stays greyed under a pack.

**Changelog.** The extension's version is `SUNRISE_HDR`'s value.

- 1: `sunrise.hdr`, `sunrise.hdr.tonemap`, `SUNRISE_HDR`, `SUNRISE_HDR_TONEMAP_PACK` and the linear `final`
  ([#6284](https://github.com/sunrise-org/sunrise/pull/6284)); the display uniforms
  ([#6487](https://github.com/sunrise-org/sunrise/pull/6487)).
