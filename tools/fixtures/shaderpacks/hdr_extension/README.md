# Frame identity: Sunrise's HDR extension

A test fixture of `tools/test_frame_identity_parity.py` (sessions `hdr_extension`, `hdr_extension_curve`,
`hdr_extension_sdr` and `hdr_extension_off`), not a look and not offered to players: a shader pack in the `shaders/`
format (GLSL 120, [shader-packs-spec.md](../../../../docs/cleanroom/shader-packs-spec.md)) that asks for Sunrise's HDR extension
([shaders.md](../../../../docs/shaders.md#sunrise-extensions-hdr)), so a change to how a served pack's `final`
reaches the display is held to the same pixels. `gbuffers_terrain` colours terrain as vanilla does (texture, tint,
lightmap), the rest of the world is the renderer's own, and `vignette = false` keeps the vignette off what the
harness reads. `final` writes, from the top of the picture:

- the top tenth: a ramp of 16 bands, 0.5, 1.0, ... 8.0 times paper white;
- the next tenth: the display in seven parts, each scaled to at most paper white so that no curve or hold reaches
  it: `sunriseHdr`, `sunriseHeadroom` / 8, `sunrisePaperWhite` / 203, `sunrisePeak` / 1000 (203 and 1000 nits are
  the harness's paper white and peak), `sunriseBlackLevel` / 0.2 (0.1 nits, the harness's black level with HDR on),
  green where `SUNRISE_HDR_TONEMAP_PACK` is defined exactly when the pack's option `SUNRISE_CURVE` is off (red
  otherwise), and last 6 times paper white, written past the pack's own hold;
- the rest: the world, decoded to light.

Served (`SUNRISE_HDR`), it writes all of it as linear light, held at `sunriseHeadroom` where the pack owns the tone
curve, the last part aside: Sunrise holds that at the display's peak, eases it with its curve, or clips it at white on
an SDR frame. Not served, it writes its SDR path: the same picture clipped at white in the 2.2 power, the display
strip black but its last part white. `SUNRISE_CURVE=true` in `shaderpacks/hdr_extension.txt` (session
`hdr_extension_curve`) gives the tone curve to Sunrise through `sunrise.hdr.tonemap`, behind the option's `#ifdef`.
