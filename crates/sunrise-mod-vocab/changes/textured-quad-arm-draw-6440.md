# `BlockEntityDrawContext::textured_quad` is refused in a first-person arm draw

`BlockEntityDrawContext::textured_quad` now returns `false`, appending nothing, in a `render_arm` draw, as it already did in a `render_hand` draw. The arm's draw joins the first-person hand's vertices. Sunrise's hand pass and a shader pack's `gbuffers_hand` draw those from the shared atlas, so an arm's textured quad showed the atlas instead of its texture. The arm draw's other calls (`quad`, `cube`, `block`, `item`) are unchanged (#6440).
