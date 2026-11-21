# Third-party notices

## cap-std / cap-fs-ext

`sunrise-client` directly uses `cap-std` and `cap-fs-ext` 3.4.6 from
[the cap-std project](https://github.com/bytecodealliance/cap-std) for confined mod file access.
Copyrights are retained by their contributors. Both crates offer Apache-2.0 WITH LLVM-exception,
Apache-2.0 or MIT; the MIT option is reproduced here:

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

## horizon-mixins

The raw injection schema in `crates/sunrise-mod-package/src/manifest.rs` is adapted from
[horizon-mixins](https://github.com/horizon-rust/horizon-mixins/tree/ddd9700b21997c95d95ac6ee5ecafe3830cf622a),
Copyright (c) 2026 levzzz, under the MIT license
(`crates/sunrise-mod-package/HORIZON_MIXINS_LICENSE`).

## Code removed for its license

Sunrise once shipped three pieces of code under MIT that came from projects whose licenses don't allow that. All
three are gone from main. They are listed here for anyone who holds an older commit, tag or release: in those
versions, that code is not covered by Horizon's MIT license. It stays under its authors' copyright and its original
license.

- **Shader packs written from Iris's source.** `crates/sunrise-shaderpack` and the render stages that used it were
  written from the source of [Iris](https://github.com/IrisShaders/Iris) (LGPL-3.0, copyright the Iris contributors).
  They were added from 15 October 2026 (`b22583d057`), shipped in the game releases `v2026.10.16.02` to `v0.2.46`,
  and were removed on 24 October 2026 ([#1561](https://github.com/horizon-rust/horizon/pull/1561)) after a developer
  of Iris showed us. Shader packs came back in `crates/sunrise-shaders`, first written from a description of the
  pack format.
- **Math functions translated from OpenJDK.** In `crates/sunrise-core/src/jmath.rs`, `fdlibm::{log, log10, cbrt,
  hypot}` were translated from OpenJDK's `java/lang/FdLibm.java` and `round_f32` from its `java/lang/Math.java`
  (GPL-2.0 with the Classpath Exception), and `aarch64::{sin, cos}` from HotSpot's `macroAssembler_aarch64_trig.cpp`
  (GPL-2.0); copyright Oracle and/or its affiliates, and Cavium for that file. The `fdlibm` and `aarch64` functions
  were added on 13 October 2026 (`558d140fca`, first released in `v2026.10.13`). `round_f32` was added on 14 October
  2026 as `java_round` in `crates/sunrise-client/src/mipmap.rs` (`95db0fce9d`) and moved into `jmath.rs` on 3
  November ([#7770](https://github.com/horizon-rust/horizon/pull/7770)). All of them were replaced in
  [#11944](https://github.com/horizon-rust/horizon/pull/11944) by code written without OpenJDK's sources, from Sun's
  fdlibm C sources, the Java SE API documentation and a description of the behaviour, and checked against a JVM
  ([#6741](https://github.com/horizon-rust/horizon/issues/6741)).
- **Java's `HashMap` translated from OpenJDK.** `crates/sunrise-world/src/java_hash_map.rs`, which reproduces the
  iteration order of `java.util.HashMap`, was translated from OpenJDK's `java/util/HashMap.java` (GPL-2.0 with the
  Classpath Exception; copyright Oracle and/or its affiliates). It was added on 7 October 2026 (`118e93ea51` and
  `194db0dbb4`, first released in `v2026.10.12`). It was replaced in
  [#12043](https://github.com/horizon-rust/horizon/pull/12043) by code written without OpenJDK's sources, from a
  description of the behaviour, the Java SE API documentation and the textbook red-black tree, and checked against a
  JVM ([#6741](https://github.com/horizon-rust/horizon/issues/6741)).

## Catnip / Ponder

`crates/sunrise-client-render/src/fluid_render.rs` translates the still-fluid face tiling and
UV rules of Catnip's `FluidRenderHelper` from Ponder 1.0.82 for Minecraft 1.21.1
(`net.createmod.ponder:ponder-neoforge:1.0.82+mc1.21.1`). Copyright (c) 2022 The Create Team,
licensed under MIT (`LICENSE_Ponder` in that artifact); the full license heads the Rust file.
The native renderer uses Sunrise's own block-sprite arrays, pose and light packing. The
development-only oracle calls the unchanged helper from the player's local artifact. No
original classes, game shaders, textures or prepared resources are distributed here.

## wgpu-hal

`third_party/wgpu-hal` is [wgpu-hal](https://github.com/gfx-rs/wgpu) 30.0.1 by the gfx-rs
developers, with upstream fixes applied
([gfx-rs/wgpu#10124](https://github.com/gfx-rs/wgpu/pull/10124),
[gfx-rs/wgpu#10714](https://github.com/gfx-rs/wgpu/pull/10714)). Its GL backend also reports
the context's sampled textures and samplers per shader stage (usually 32 on desktop GL,
where WebGL2's minimum 16 was reported before), so a pipeline layout the driver accepts is
not refused. Its GL backend copies depth and stencil textures with a framebuffer blit, where
`glCopyTexSubImage` read only colour and left the copy empty. It is licensed under MIT or
Apache-2.0; both licenses are in that folder
(`LICENSE.MIT`, `LICENSE.APACHE`).

## naga

`third_party/naga` is [naga](https://github.com/gfx-rs/wgpu) 30.0.1 by the gfx-rs developers, with
fixes to its GLSL frontend that shader packs need, each applied in the commit that needed it. It is licensed under MIT or Apache-2.0; both licenses
are in that folder (`LICENSE.MIT`, `LICENSE.APACHE`).

The GLSL frontend also resets the common type for each comma-separated declarator so one
variable's array specifier does not change later variables.
It also reads a struct member outside a constant expression through a pointer to the member,
not out of a loaded copy of the whole struct, so a member an atomic function uses can be read.
The HLSL backend writes an ordinary level-zero texture sample as `SampleLevel(..., 0.0)`, while
comparison textures keep `SampleCmpLevelZero`; HLSL has no ordinary `SampleLevelZero` method.
The shared interpolation defaults also compile with `glsl-in` alone, without needing the
SPIR-V or WGSL frontend to enable them.
The HLSL backend's texture-sampling offset cast preserves the offset's scalar or vector width,
including `int` for 1D and `int3` for 3D textures instead of always using `int2`.

## DirectX Shader Compiler

The Windows x64 game folder holds `dxcompiler.dll`, Microsoft's DirectX Shader Compiler
[v1.9.2609](https://github.com/microsoft/DirectXShaderCompiler/releases/tag/v1.9.2609), unmodified: the
`bin/x64/dxcompiler.dll` of `dxc_2026_09_29.zip` (zip SHA-256
`ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1`, DLL SHA-256
`0689e2fee0a1c9df8662a59680469f24e2a3ad7ce6bbbcc9284f7a83172bfc59`, 22,274,872 bytes). The release build downloads
it and checks both checksums (`tools/ci/fetch_dxc.py`); it is not in this repository. DirectX 12 loads it only on
integrated GPUs (`crates/sunrise-client-render/src/gfx/dx12_compiler.rs`,
[#8513](https://github.com/horizon-rust/horizon/issues/8513)); every other GPU compiles with FXC, part of Windows.

Copyright (c) Microsoft Corporation. It is not covered by Horizon's MIT license. It is distributed under the
"Microsoft Software License Terms, Microsoft DirectX Shader Compiler" (`LICENSE-MS.txt` of that release). Its
Distributable Code terms are the Windows SDK's: end users get terms that protect the code and Microsoft at least as
much, and the distributor indemnifies Microsoft. The project accepted them on 2026-11-08. Its open-source part is also under the University of Illinois/NCSA license (`LICENSE-LLVM.txt`). Both
files ship beside the DLL as `DXC-LICENSE-MS.txt` and `DXC-LICENSE-LLVM.txt`, with `DXC-NOTICE.txt`
(`packaging/windows/`), which tells players those terms apply and that deleting the DLL brings back FXC.

- Only x64: the Windows SDK's list of distributable files
  ([REDIST.TXT lists](https://learn.microsoft.com/en-us/legal/windows-sdk/redist), updated 2024-10-21) names
  `Redist\D3D\x64\dxcompiler.dll` and `dxil.dll` and their x86 copies, for classic Windows applications (Sunrise is
  one), and no ARM64 copy. The release zip's own `LICENSE-MS.txt` refers to a list of distributable files that the zip
  doesn't contain, so the SDK's list is the one followed. The Windows ARM64 build ships no DXC and compiles with FXC.
- No `dxil.dll`: since DXC 1.8.2505 the compiler signs shaders with its own validator and no longer loads it, and
  wgpu never does.

## libmimalloc-sys and mimalloc

`third_party/libmimalloc-sys` is [libmimalloc-sys](https://github.com/purpleprotocol/mimalloc_rust)
0.1.49 by Octavian Oncescu, with the [mimalloc](https://github.com/microsoft/mimalloc) v3 sources it
bundles (Copyright (c) 2018-2025 Microsoft Corporation, Daan Leijen), both under the MIT license
(`LICENSE.txt` and `c_src/mimalloc/v3/LICENSE` in that folder). It holds only the v3 sources. Three
changes: `mi_popcount` in `include/mimalloc/bits.h` uses mimalloc's portable bit count on MSVC
x86/x64 instead of the POPCNT instruction, which CPUs before Nehalem lack (#7264);
`build.rs` reruns when a file under `c_src/mimalloc/v3` changes, so a kept build directory never links stale
sources (#3299); and
`_mi_arenas_page_free` in `src/arena.c` lowers the `malloc_huge` statistic when a huge page is freed, at
every `MI_STAT` level as `src/page.c` raises it, instead of `mi_stat_free` (`src/free.c`) and
`_mi_theap_page_destroy` (`src/theap.c`) lowering it only with detailed statistics (#3299).

## Pixelify Sans

`crates/sunrise-theme/fonts/PixelifySans-*.ttf` is [Pixelify Sans](https://github.com/eifetx/Pixelify-Sans)
by The Pixelify Sans Project Authors, licensed under the SIL Open Font License 1.1
(`crates/sunrise-theme/fonts/OFL-PixelifySans.txt`).

## Pixelarticons

`crates/sunrise-theme/icons/pixel/*.svg` are icons from
[Pixelarticons](https://github.com/halfmage/pixelarticons) by Gerrit Halfmann, under the MIT
license (`crates/sunrise-theme/icons/pixel/LICENSE-pixelarticons.txt`). `star.svg` is Sunrise's own, drawn on
the same grid.

## Hugeicons

`crates/sunrise-theme/icons/glass/*.svg` are icons from the free set of
[Hugeicons](https://github.com/hugeicons/hugeicons) (`@hugeicons/core-free-icons` 4.4.0), under
the MIT license (`crates/sunrise-theme/icons/glass/LICENSE-hugeicons.txt`), exported with a 1.6
stroke in `currentColor`.

`crates/sunrise-theme/icons/glass/discord.svg` is Discord's logo as drawn by
[Simple Icons](https://github.com/simple-icons/simple-icons) 13.21.0 (CC0 1.0). The logo is Discord's
trademark, used only on the button that opens Sunrise's Discord server.

## AMD FidelityFX SDK (FSR 3.1 frame generation)

`crates/sunrise-client-render/src/gfx/frame_gen/optical_flow.wgsl` and `interpolation.wgsl` are
ports to WGSL of the optical flow and frame interpolation shaders of the
[AMD FidelityFX SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) 1.1.4
(`sdk/include/FidelityFX/gpu/{opticalflow,frameinterpolation}`, at commit
[c6efa6bf7f20](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/tree/c6efa6bf7f2027b3ec94f28578bb5965eabb9e55)),
and `optical_flow.rs` and `interpolation.rs` follow the SDK's
`sdk/src/components/{opticalflow/ffx_opticalflow.cpp,frameinterpolation/ffx_frameinterpolation.cpp}`
(their passes, their order and their dispatch sizes). Copyright (C) 2024 Advanced Micro Devices,
Inc., under the MIT license, whose text heads each file. AMD and FidelityFX are AMD's
trademarks; Sunrise is not affiliated with AMD.

## Mod loader logos

The launcher shows the logo of the mod loader an instance plays with, and offers the logos as
instance icons.

- `assets/logos/{fabric,quilt,neoforge,forge}.png` in [sunrise-launcher](https://github.com/sunrise-org/sunrise-launcher) are the loaders' logos as the launcher
  mockup on [playsunrise.net/launcher](https://playsunrise.net/launcher) shows them (sunrise-site
  `public/media/launcher/*.webp`, converted to PNG), each its project's own mark:
  [Fabric](https://fabricmc.net/), [Quilt](https://quiltmc.org/) (its art is
  [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/)),
  [NeoForged](https://neoforged.net/) (its art, by Ridanisaurus, is
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)) and
  [Minecraft Forge](https://minecraftforge.net/). They name the loader an instance runs.

Each logo belongs to its project. Showing it says which loader an instance runs; it does not mean
the project endorses Sunrise.

## Sign in with Microsoft

The launcher's sign-in buttons follow Microsoft's published branding guidelines for apps that sign in
with Microsoft accounts
([Sign in with Microsoft branding](https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps)).
The four-square Microsoft symbol in `https://github.com/sunrise-org/sunrise-launcher/blob/main/src/microsoft.rs` reproduces the squares, positions and
colours of Microsoft's official `ms-symbollockup_mssymbol_19.svg` from that page, unmodified. Microsoft
and its logo are trademarks of the Microsoft group of companies; Sunrise is not endorsed by Microsoft.

## CIT Resewn

`crates/sunrise-optifine/src/cit.rs` and `crates/sunrise-client-render/src/cit.rs` follow the
condition semantics of [CIT Resewn Continuation](https://github.com/EthanVisagie/CIT-Resewn-Continuation-1.21.11)
(at `b792f9a`), a continuation of [CIT Resewn](https://github.com/SHsuperCM/CITResewn) by SHsuperCM: the
1.21 component paths, enchantment levels, percent damage, text matching and rule ordering. It is
licensed under the MIT license:

> Copyright (c) 2021 SHsuperCM
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software
> and associated documentation files (the "Software"), to deal in the Software without
> restriction, including without limitation the rights to use, copy, modify, merge, publish,
> distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
> Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or
> substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
> BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
> NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
> DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## FusionExamples

The tile tables in `crates/sunrise-optifine/src/fusion/layout.rs` were read off the connecting
texture templates (`templates/template <layout>.png`) of
[FusionExamples](https://github.com/SuperMartijn642/FusionExamples) (at `8c9c56e`), which is
licensed under the MIT license:

> Copyright (c) 2024 Martijn van den Brand
>
> Permission is hereby granted, free of charge, to any person obtaining a copy of this software
> and associated documentation files (the "Software"), to deal in the Software without
> restriction, including without limitation the rights to use, copy, modify, merge, publish,
> distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
> Software is furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all copies or
> substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
> BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
> NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
> DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## BlueMap

`crates/sunrise-client-render/src/bluemap.rs` reads the lowres tiles of a server's
[BlueMap](https://github.com/BlueMap-Minecraft/BlueMap) web map. It follows three of BlueMap's files:
`core/src/main/java/de/bluecolored/bluemap/core/map/lowres/LowresLayer.java` (`saveTile`) for the
tiles' pixel layout, `common/webapp/src/js/util/Utils.js` (`pathFromCoords`) for their paths, and
`core/src/main/java/de/bluecolored/bluemap/core/map/MapSettingsSerializer.java` for a map's
`settings.json`, at commit
[96ae320d1d39](https://github.com/BlueMap-Minecraft/BlueMap/tree/96ae320d1d39581c10058d8222f2290f2fcb5333).
BlueMap is licensed under the MIT license:

> Copyright (c) Blue (Lukas Rieger) <https://bluecolored.de>
> Copyright (c) contributors
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in
> all copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
> THE SOFTWARE.

## JOML

`crates/sunrise-client-render/src/camera.rs` (`joml::Mat4::rotation`, `invert_orthonormal`) and
`crates/sunrise-client-render/src/gfx/pack_uniforms.rs` (`invert_generic`, `invert_affine`, the
formulas of `pack_projection` and `shadow_ortho`) adapt the float arithmetic of
`Matrix4f.rotation(Quaternionfc)`, `invertOrthonormal`, `invertGenericNonThis`, `invertAffine`,
`setPerspective` and `setOrtho` from
[JOML](https://github.com/JOML-CI/JOML/tree/faf9eef2511242122beb6055fe3da92daed214c5) 1.10.9, the
library vanilla 26.3 uses, so that shader packs get the same matrices to the last bit. Changes:
ported to Rust; the pack_uniforms.rs functions drop JOML's property flags, and
`pack_projection` works the formula in doubles. The copyright line and license notice also head
both files.

`camera.rs` also adapts `Quaternionf.rotationX`, `rotationY` and `rotationZ`,
`Matrix4f.rotateAffine` and `Vector4f.mulAffine` (`joml::Quat::rotation_x`/`_y`/`_z`,
`joml::Mat4::rotate_affine`, `transform_affine`), for the sky's positions shader packs read.
Changes: ported to Rust; `rotate_affine` keeps only the affine case.

`camera.rs` also adapts `Quaternionf.rotationZYX`, `Matrix4f.rotate(Quaternionfc)` (choosing
`rotation`, `rotateTranslation` or `rotateAffine` as JOML does), `Matrix4f.scale` and
`Matrix4f.transformPosition` (`joml::Quat::rotation_zyx`, `joml::Mat4::rotate`, `scale`,
`transform_position`), for the poses block entity renderers hand to the item renderer.
Changes: ported to Rust.

```
The MIT License

Copyright (c) 2015-2026 Richard Greenlees

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## Just Enough Items

The Recipes GUI sprites and settings icon under `crates/sunrise-client-render/src/jei_recipes` are from
[Just Enough Items 31.17.0.108](https://github.com/mezz/JustEnoughItems/tree/0371fa6fafa14635a356019e4604f5580b26a34f)
by mezz and contributors, under the MIT license in that directory (`LICENSE.txt`).

## Flywheel native model directional shading

The two-light diffuse expression and transformed-normal policy in
`terrain.wgsl`'s native model entries follow Flywheel 1.0.6
(`internal/diffuse.glsl`, `instance/transformed.vert`, `internal/common.vert`).
No Flywheel classes, compiled shaders or original model/texture assets are bundled.

Copyright (c) 2021-2024 Jozufozu

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## zlib

`crates/sunrise-deflate` is a Rust port of parts of [zlib](https://zlib.net/) 1.3.2 (`deflate.c` and
`trees.c`, from the official `zlib-1.3.2.tar.gz`, SHA-256
`bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16`), altered to cover only level 6 with
the default strategy, so that saves carry the bytes the JDK's bundled zlib writes. It is under the zlib
license; its notice and what was altered are in `crates/sunrise-deflate/LICENSE` and the header of each
ported file:

```
(C) 1995-2026 Jean-loup Gailly and Mark Adler
```

## fdlibm

`crates/sunrise-core/src/jmath/fdlibm.rs` and the `fdlibm_atan` and `fdlibm_atan2`
functions in `crates/sunrise-core/src/geometry/trig.rs` are Rust ports of
[fdlibm](https://www.netlib.org/fdlibm/) 5.3, Sun's freely distributable math library, whose
results the Java SE API specifies for `StrictMath`. `fdlibm.rs` ports `e_log.c`, `e_log10.c`,
`s_cbrt.c` and `e_hypot.c`, and, with fused multiply-adds, `s_sin.c`, `s_cos.c`, `k_sin.c`,
`k_cos.c` and `e_rem_pio2.c`; `trig.rs` ports `s_atan.c` and `e_atan2.c`.

The strict additions in `crates/sunrise-core/src/jmath/fdlibm/strict_exp_pow.rs`
adapt netlib's original `e_exp.c` 1.6 (2004-04-22), `e_pow.c` 1.5
(2004-04-22) and `s_scalbn.c` 1.3 (1995-01-18), preserving their ordered
double arithmetic and Sun permission notices. Their public-JDK reference
vectors are generated by `tools/generate_strict_math_vectors.py` through
`StrictMath` API calls; no JDK/OpenJDK implementation source was used.

`strict_trig.rs` adapts original netlib `s_sin.c`, `s_cos.c`, `s_tan.c`,
`k_sin.c`, `k_cos.c`, `k_tan.c`, `e_rem_pio2.c` and `k_rem_pio2.c`.
The full large-argument reduction kernel specializes its original `prec=2`
double-precision path; the unfused arithmetic order is preserved. The tangent
kernel carries Sun's 2004 notice; the other trigonometric sources carry Sun's
1993 notice.

The original Sun notice is preserved in each adapted source:

```
Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

Developed at SunSoft, a Sun Microsystems, Inc. business.
Permission to use, copy, modify, and distribute this
software is freely granted, provided that this notice
is preserved.
```

## Intel's libm

`crates/sunrise-core/src/jmath/intel_trig.rs` is a Rust translation of Intel's libm: `s_sin.S`, `s_cos.S` and
`s_tan.S` from [bionic's `libm/x86_64`](https://android.googlesource.com/platform/bionic/+/refs/tags/android-9.0.0_r1/libm/x86_64/)
(tag android-9.0.0_r1), the algorithms HotSpot runs for `Math.sin`, `Math.cos` and `Math.tan` on x86-64. Changes:
translated from x86-64 assembly to Rust, with the same floating-point operations in the same order (the packed SSE2
ones through the `safe_arch` crate) and the tables as typed Rust arrays. The copyright line and license notice also
head the file.

```
Copyright (c) 2014, Intel Corporation
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright notice,
    * this list of conditions and the following disclaimer.

    * Redistributions in binary form must reproduce the above copyright notice,
    * this list of conditions and the following disclaimer in the documentation
    * and/or other materials provided with the distribution.

    * Neither the name of Intel Corporation nor the names of its contributors
    * may be used to endorse or promote products derived from this software
    * without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## PCG3D hash

`hash3` in `crates/sunrise-client-render/src/hdr.wgsl`, the noise HDR10 output's dither is made from, is the
`pcg3d` hash of Mark Jarzynski and Marc Olano ("Hash Functions for GPU Rendering", Journal of Computer Graphics
Techniques 9(3), 2020), from <https://github.com/markjarzynski/PCG3D>. Copyright 2020 Mark Jarzynski & Marc Olano,
licensed under MIT:

```
Copyright 2020 Mark Jarzynski & Marc Olano

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## OpenAL Soft HRTF data set

`crates/sunrise-client-render/assets/hrtf/Default HRTF.mhr` is the HRTF data set of
[OpenAL Soft](https://github.com/kcat/openal-soft) (its `Built-In HRTF`), taken unmodified from tag
`1.25.2` (commit `b2c48f7718ef`), path `hrtf/Default HRTF.mhr`, 159,841 bytes. Directional Audio renders
through it as vanilla's bundled OpenAL Soft does (#3511); the file is data, loaded by Sunrise's own
code, and no OpenAL Soft source is used. Its notice, followed by the text of OpenAL Soft's `COPYING`
at that tag (SHA-256 `d808ce217e5b611854da622b57ec29fe545584c48bc5352fae72a4b6e5074a15`; one line of
spaces there is empty here):

```
Default HRTF.mhr
HRTF data set from OpenAL Soft 1.25.2 (https://github.com/kcat/openal-soft,
tag 1.25.2, file hrtf/Default HRTF.mhr), unmodified.
SHA-256 0b2f09f4d9167dec4e977e7e1a8c78df17e87da06fb0871b9896fadaa473a440

OpenAL cross platform audio library
Copyright (C) 1999-2007 by authors.
This library is free software; you can redistribute it and/or
modify it under the terms of the GNU Library General Public
License as published by the Free Software Foundation; either
version 2 of the License, or (at your option) any later version.
This library is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
Library General Public License for more details.
The full text of the GNU Library General Public License, version 2,
follows below.

The data set is derived from the KEMAR HRTF measurements by
Bill Gardner and Keith Martin, MIT Media Lab Machine Listening Group,
"HRTF Measurements of a KEMAR Dummy-Head Microphone" (1994),
http://sound.media.mit.edu/resources/KEMAR.html.
This data is Copyright 1994 by the MIT Media Laboratory. It is
provided free with no restrictions on use, provided the authors
are cited when the data is used in any research or commercial
application.
```

```
                  GNU LIBRARY GENERAL PUBLIC LICENSE
                       Version 2, June 1991

 Copyright (C) 1991 Free Software Foundation, Inc.
 51 Franklin Street, Fifth Floor, Boston, MA  02110-1301  USA
 Everyone is permitted to copy and distribute verbatim copies
 of this license document, but changing it is not allowed.

[This is the first released version of the library GPL.  It is
 numbered 2 because it goes with version 2 of the ordinary GPL.]

                            Preamble

  The licenses for most software are designed to take away your
freedom to share and change it.  By contrast, the GNU General Public
Licenses are intended to guarantee your freedom to share and change
free software--to make sure the software is free for all its users.

  This license, the Library General Public License, applies to some
specially designated Free Software Foundation software, and to any
other libraries whose authors decide to use it.  You can use it for
your libraries, too.

  When we speak of free software, we are referring to freedom, not
price.  Our General Public Licenses are designed to make sure that you
have the freedom to distribute copies of free software (and charge for
this service if you wish), that you receive source code or can get it
if you want it, that you can change the software or use pieces of it
in new free programs; and that you know you can do these things.

  To protect your rights, we need to make restrictions that forbid
anyone to deny you these rights or to ask you to surrender the rights.
These restrictions translate to certain responsibilities for you if
you distribute copies of the library, or if you modify it.

  For example, if you distribute copies of the library, whether gratis
or for a fee, you must give the recipients all the rights that we gave
you.  You must make sure that they, too, receive or can get the source
code.  If you link a program with the library, you must provide
complete object files to the recipients so that they can relink them
with the library, after making changes to the library and recompiling
it.  And you must show them these terms so they know their rights.

  Our method of protecting your rights has two steps: (1) copyright
the library, and (2) offer you this license which gives you legal
permission to copy, distribute and/or modify the library.

  Also, for each distributor's protection, we want to make certain
that everyone understands that there is no warranty for this free
library.  If the library is modified by someone else and passed on, we
want its recipients to know that what they have is not the original
version, so that any problems introduced by others will not reflect on
the original authors' reputations.

  Finally, any free program is threatened constantly by software
patents.  We wish to avoid the danger that companies distributing free
software will individually obtain patent licenses, thus in effect
transforming the program into proprietary software.  To prevent this,
we have made it clear that any patent must be licensed for everyone's
free use or not licensed at all.

  Most GNU software, including some libraries, is covered by the ordinary
GNU General Public License, which was designed for utility programs.  This
license, the GNU Library General Public License, applies to certain
designated libraries.  This license is quite different from the ordinary
one; be sure to read it in full, and don't assume that anything in it is
the same as in the ordinary license.

  The reason we have a separate public license for some libraries is that
they blur the distinction we usually make between modifying or adding to a
program and simply using it.  Linking a program with a library, without
changing the library, is in some sense simply using the library, and is
analogous to running a utility program or application program.  However, in
a textual and legal sense, the linked executable is a combined work, a
derivative of the original library, and the ordinary General Public License
treats it as such.

  Because of this blurred distinction, using the ordinary General
Public License for libraries did not effectively promote software
sharing, because most developers did not use the libraries.  We
concluded that weaker conditions might promote sharing better.

  However, unrestricted linking of non-free programs would deprive the
users of those programs of all benefit from the free status of the
libraries themselves.  This Library General Public License is intended to
permit developers of non-free programs to use free libraries, while
preserving your freedom as a user of such programs to change the free
libraries that are incorporated in them.  (We have not seen how to achieve
this as regards changes in header files, but we have achieved it as regards
changes in the actual functions of the Library.)  The hope is that this
will lead to faster development of free libraries.

  The precise terms and conditions for copying, distribution and
modification follow.  Pay close attention to the difference between a
"work based on the library" and a "work that uses the library".  The
former contains code derived from the library, while the latter only
works together with the library.

  Note that it is possible for a library to be covered by the ordinary
General Public License rather than by this special one.

                  GNU LIBRARY GENERAL PUBLIC LICENSE
   TERMS AND CONDITIONS FOR COPYING, DISTRIBUTION AND MODIFICATION

  0. This License Agreement applies to any software library which
contains a notice placed by the copyright holder or other authorized
party saying it may be distributed under the terms of this Library
General Public License (also called "this License").  Each licensee is
addressed as "you".

  A "library" means a collection of software functions and/or data
prepared so as to be conveniently linked with application programs
(which use some of those functions and data) to form executables.

  The "Library", below, refers to any such software library or work
which has been distributed under these terms.  A "work based on the
Library" means either the Library or any derivative work under
copyright law: that is to say, a work containing the Library or a
portion of it, either verbatim or with modifications and/or translated
straightforwardly into another language.  (Hereinafter, translation is
included without limitation in the term "modification".)

  "Source code" for a work means the preferred form of the work for
making modifications to it.  For a library, complete source code means
all the source code for all modules it contains, plus any associated
interface definition files, plus the scripts used to control compilation
and installation of the library.

  Activities other than copying, distribution and modification are not
covered by this License; they are outside its scope.  The act of
running a program using the Library is not restricted, and output from
such a program is covered only if its contents constitute a work based
on the Library (independent of the use of the Library in a tool for
writing it).  Whether that is true depends on what the Library does
and what the program that uses the Library does.

  1. You may copy and distribute verbatim copies of the Library's
complete source code as you receive it, in any medium, provided that
you conspicuously and appropriately publish on each copy an
appropriate copyright notice and disclaimer of warranty; keep intact
all the notices that refer to this License and to the absence of any
warranty; and distribute a copy of this License along with the
Library.

  You may charge a fee for the physical act of transferring a copy,
and you may at your option offer warranty protection in exchange for a
fee.

  2. You may modify your copy or copies of the Library or any portion
of it, thus forming a work based on the Library, and copy and
distribute such modifications or work under the terms of Section 1
above, provided that you also meet all of these conditions:

    a) The modified work must itself be a software library.

    b) You must cause the files modified to carry prominent notices
    stating that you changed the files and the date of any change.

    c) You must cause the whole of the work to be licensed at no
    charge to all third parties under the terms of this License.

    d) If a facility in the modified Library refers to a function or a
    table of data to be supplied by an application program that uses
    the facility, other than as an argument passed when the facility
    is invoked, then you must make a good faith effort to ensure that,
    in the event an application does not supply such function or
    table, the facility still operates, and performs whatever part of
    its purpose remains meaningful.

    (For example, a function in a library to compute square roots has
    a purpose that is entirely well-defined independent of the
    application.  Therefore, Subsection 2d requires that any
    application-supplied function or table used by this function must
    be optional: if the application does not supply it, the square
    root function must still compute square roots.)

These requirements apply to the modified work as a whole.  If
identifiable sections of that work are not derived from the Library,
and can be reasonably considered independent and separate works in
themselves, then this License, and its terms, do not apply to those
sections when you distribute them as separate works.  But when you
distribute the same sections as part of a whole which is a work based
on the Library, the distribution of the whole must be on the terms of
this License, whose permissions for other licensees extend to the
entire whole, and thus to each and every part regardless of who wrote
it.

Thus, it is not the intent of this section to claim rights or contest
your rights to work written entirely by you; rather, the intent is to
exercise the right to control the distribution of derivative or
collective works based on the Library.

In addition, mere aggregation of another work not based on the Library
with the Library (or with a work based on the Library) on a volume of
a storage or distribution medium does not bring the other work under
the scope of this License.

  3. You may opt to apply the terms of the ordinary GNU General Public
License instead of this License to a given copy of the Library.  To do
this, you must alter all the notices that refer to this License, so
that they refer to the ordinary GNU General Public License, version 2,
instead of to this License.  (If a newer version than version 2 of the
ordinary GNU General Public License has appeared, then you can specify
that version instead if you wish.)  Do not make any other change in
these notices.

  Once this change is made in a given copy, it is irreversible for
that copy, so the ordinary GNU General Public License applies to all
subsequent copies and derivative works made from that copy.

  This option is useful when you wish to copy part of the code of
the Library into a program that is not a library.

  4. You may copy and distribute the Library (or a portion or
derivative of it, under Section 2) in object code or executable form
under the terms of Sections 1 and 2 above provided that you accompany
it with the complete corresponding machine-readable source code, which
must be distributed under the terms of Sections 1 and 2 above on a
medium customarily used for software interchange.

  If distribution of object code is made by offering access to copy
from a designated place, then offering equivalent access to copy the
source code from the same place satisfies the requirement to
distribute the source code, even though third parties are not
compelled to copy the source along with the object code.

  5. A program that contains no derivative of any portion of the
Library, but is designed to work with the Library by being compiled or
linked with it, is called a "work that uses the Library".  Such a
work, in isolation, is not a derivative work of the Library, and
therefore falls outside the scope of this License.

  However, linking a "work that uses the Library" with the Library
creates an executable that is a derivative of the Library (because it
contains portions of the Library), rather than a "work that uses the
library".  The executable is therefore covered by this License.
Section 6 states terms for distribution of such executables.

  When a "work that uses the Library" uses material from a header file
that is part of the Library, the object code for the work may be a
derivative work of the Library even though the source code is not.
Whether this is true is especially significant if the work can be
linked without the Library, or if the work is itself a library.  The
threshold for this to be true is not precisely defined by law.

  If such an object file uses only numerical parameters, data
structure layouts and accessors, and small macros and small inline
functions (ten lines or less in length), then the use of the object
file is unrestricted, regardless of whether it is legally a derivative
work.  (Executables containing this object code plus portions of the
Library will still fall under Section 6.)

  Otherwise, if the work is a derivative of the Library, you may
distribute the object code for the work under the terms of Section 6.
Any executables containing that work also fall under Section 6,
whether or not they are linked directly with the Library itself.

  6. As an exception to the Sections above, you may also compile or
link a "work that uses the Library" with the Library to produce a
work containing portions of the Library, and distribute that work
under terms of your choice, provided that the terms permit
modification of the work for the customer's own use and reverse
engineering for debugging such modifications.

  You must give prominent notice with each copy of the work that the
Library is used in it and that the Library and its use are covered by
this License.  You must supply a copy of this License.  If the work
during execution displays copyright notices, you must include the
copyright notice for the Library among them, as well as a reference
directing the user to the copy of this License.  Also, you must do one
of these things:

    a) Accompany the work with the complete corresponding
    machine-readable source code for the Library including whatever
    changes were used in the work (which must be distributed under
    Sections 1 and 2 above); and, if the work is an executable linked
    with the Library, with the complete machine-readable "work that
    uses the Library", as object code and/or source code, so that the
    user can modify the Library and then relink to produce a modified
    executable containing the modified Library.  (It is understood
    that the user who changes the contents of definitions files in the
    Library will not necessarily be able to recompile the application
    to use the modified definitions.)

    b) Accompany the work with a written offer, valid for at
    least three years, to give the same user the materials
    specified in Subsection 6a, above, for a charge no more
    than the cost of performing this distribution.

    c) If distribution of the work is made by offering access to copy
    from a designated place, offer equivalent access to copy the above
    specified materials from the same place.

    d) Verify that the user has already received a copy of these
    materials or that you have already sent this user a copy.

  For an executable, the required form of the "work that uses the
Library" must include any data and utility programs needed for
reproducing the executable from it.  However, as a special exception,
the source code distributed need not include anything that is normally
distributed (in either source or binary form) with the major
components (compiler, kernel, and so on) of the operating system on
which the executable runs, unless that component itself accompanies
the executable.

  It may happen that this requirement contradicts the license
restrictions of other proprietary libraries that do not normally
accompany the operating system.  Such a contradiction means you cannot
use both them and the Library together in an executable that you
distribute.

  7. You may place library facilities that are a work based on the
Library side-by-side in a single library together with other library
facilities not covered by this License, and distribute such a combined
library, provided that the separate distribution of the work based on
the Library and of the other library facilities is otherwise
permitted, and provided that you do these two things:

    a) Accompany the combined library with a copy of the same work
    based on the Library, uncombined with any other library
    facilities.  This must be distributed under the terms of the
    Sections above.

    b) Give prominent notice with the combined library of the fact
    that part of it is a work based on the Library, and explaining
    where to find the accompanying uncombined form of the same work.

  8. You may not copy, modify, sublicense, link with, or distribute
the Library except as expressly provided under this License.  Any
attempt otherwise to copy, modify, sublicense, link with, or
distribute the Library is void, and will automatically terminate your
rights under this License.  However, parties who have received copies,
or rights, from you under this License will not have their licenses
terminated so long as such parties remain in full compliance.

  9. You are not required to accept this License, since you have not
signed it.  However, nothing else grants you permission to modify or
distribute the Library or its derivative works.  These actions are
prohibited by law if you do not accept this License.  Therefore, by
modifying or distributing the Library (or any work based on the
Library), you indicate your acceptance of this License to do so, and
all its terms and conditions for copying, distributing or modifying
the Library or works based on it.

  10. Each time you redistribute the Library (or any work based on the
Library), the recipient automatically receives a license from the
original licensor to copy, distribute, link with or modify the Library
subject to these terms and conditions.  You may not impose any further
restrictions on the recipients' exercise of the rights granted herein.
You are not responsible for enforcing compliance by third parties to
this License.

  11. If, as a consequence of a court judgment or allegation of patent
infringement or for any other reason (not limited to patent issues),
conditions are imposed on you (whether by court order, agreement or
otherwise) that contradict the conditions of this License, they do not
excuse you from the conditions of this License.  If you cannot
distribute so as to satisfy simultaneously your obligations under this
License and any other pertinent obligations, then as a consequence you
may not distribute the Library at all.  For example, if a patent
license would not permit royalty-free redistribution of the Library by
all those who receive copies directly or indirectly through you, then
the only way you could satisfy both it and this License would be to
refrain entirely from distribution of the Library.

If any portion of this section is held invalid or unenforceable under any
particular circumstance, the balance of the section is intended to apply,
and the section as a whole is intended to apply in other circumstances.

It is not the purpose of this section to induce you to infringe any
patents or other property right claims or to contest validity of any
such claims; this section has the sole purpose of protecting the
integrity of the free software distribution system which is
implemented by public license practices.  Many people have made
generous contributions to the wide range of software distributed
through that system in reliance on consistent application of that
system; it is up to the author/donor to decide if he or she is willing
to distribute software through any other system and a licensee cannot
impose that choice.

This section is intended to make thoroughly clear what is believed to
be a consequence of the rest of this License.

  12. If the distribution and/or use of the Library is restricted in
certain countries either by patents or by copyrighted interfaces, the
original copyright holder who places the Library under this License may add
an explicit geographical distribution limitation excluding those countries,
so that distribution is permitted only in or among countries not thus
excluded.  In such case, this License incorporates the limitation as if
written in the body of this License.

  13. The Free Software Foundation may publish revised and/or new
versions of the Library General Public License from time to time.
Such new versions will be similar in spirit to the present version,
but may differ in detail to address new problems or concerns.

Each version is given a distinguishing version number.  If the Library
specifies a version number of this License which applies to it and
"any later version", you have the option of following the terms and
conditions either of that version or of any later version published by
the Free Software Foundation.  If the Library does not specify a
license version number, you may choose any version ever published by
the Free Software Foundation.

  14. If you wish to incorporate parts of the Library into other free
programs whose distribution conditions are incompatible with these,
write to the author to ask for permission.  For software which is
copyrighted by the Free Software Foundation, write to the Free
Software Foundation; we sometimes make exceptions for this.  Our
decision will be guided by the two goals of preserving the free status
of all derivatives of our free software and of promoting the sharing
and reuse of software generally.

                            NO WARRANTY

  15. BECAUSE THE LIBRARY IS LICENSED FREE OF CHARGE, THERE IS NO
WARRANTY FOR THE LIBRARY, TO THE EXTENT PERMITTED BY APPLICABLE LAW.
EXCEPT WHEN OTHERWISE STATED IN WRITING THE COPYRIGHT HOLDERS AND/OR
OTHER PARTIES PROVIDE THE LIBRARY "AS IS" WITHOUT WARRANTY OF ANY
KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE.  THE ENTIRE RISK AS TO THE QUALITY AND PERFORMANCE OF THE
LIBRARY IS WITH YOU.  SHOULD THE LIBRARY PROVE DEFECTIVE, YOU ASSUME
THE COST OF ALL NECESSARY SERVICING, REPAIR OR CORRECTION.

  16. IN NO EVENT UNLESS REQUIRED BY APPLICABLE LAW OR AGREED TO IN
WRITING WILL ANY COPYRIGHT HOLDER, OR ANY OTHER PARTY WHO MAY MODIFY
AND/OR REDISTRIBUTE THE LIBRARY AS PERMITTED ABOVE, BE LIABLE TO YOU
FOR DAMAGES, INCLUDING ANY GENERAL, SPECIAL, INCIDENTAL OR
CONSEQUENTIAL DAMAGES ARISING OUT OF THE USE OR INABILITY TO USE THE
LIBRARY (INCLUDING BUT NOT LIMITED TO LOSS OF DATA OR DATA BEING
RENDERED INACCURATE OR LOSSES SUSTAINED BY YOU OR THIRD PARTIES OR A
FAILURE OF THE LIBRARY TO OPERATE WITH ANY OTHER SOFTWARE), EVEN IF
SUCH HOLDER OR OTHER PARTY HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
DAMAGES.

                     END OF TERMS AND CONDITIONS
```
