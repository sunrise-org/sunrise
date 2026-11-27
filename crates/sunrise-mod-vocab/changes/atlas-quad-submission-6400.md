# `BlockEntityDrawContext::quad` reports whether it appended the quad

`BlockEntityDrawContext::quad` now returns whether it appended a quad, rather than whether its atlas sprite resolved. Unknown sprites, non-finite submitted positions or UVs, and a full batch return `false` without appending geometry (#6400).
