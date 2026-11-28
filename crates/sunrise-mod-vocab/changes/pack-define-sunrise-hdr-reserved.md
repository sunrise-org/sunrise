# Pack defines: Sunrise's HDR names are reserved

`PackBinding::validate_syntax` now refuses a pack define named `SUNRISE_HDR` or `SUNRISE_HDR_TONEMAP_PACK`, as it
refuses `GL_` names. The game defines them for a `shaders/` pack only when it serves Sunrise's HDR extension to it, so
a mod can't make a pack take its HDR path on a frame that wasn't made for it.
