# `ShaderPackState::generation` continues across renderer rebuilds

`ShaderPackState::generation` now continues across renderer rebuilds — same-device resource reloads, frame-shape rebuilds and graphics device recovery — instead of restarting at `0` on the replacement renderer. Before, switching shader packs across a renderer rebuild could report the same generation as the previous pack. The struct and query signatures are unchanged.

Mods that cache per-pack derived state keyed on `ShaderPackState::generation` (and `None` when inactive) no longer see a generation collision after a rebuild.
