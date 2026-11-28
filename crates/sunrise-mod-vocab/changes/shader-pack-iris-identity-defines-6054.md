# Pack defines may not name IS_IRIS or IRIS_VERSION

`PackBinding::validate_syntax` now refuses a define named `IS_IRIS` or `IRIS_VERSION`, as it refuses `GL_` and
`IRIS_FEATURE_*` names: registration refuses a define it used to accept. Before, registration accepted such a define and
the renderer passed it to the shader pack. The engine now defines these two macros itself, for a pack whose Iris
compatibility is on (Sunrise Settings › Shaders: Auto, On or Off), and only then; a mod's define would turn a pack's Iris
path on behind the player's switch. A mod that declared one now gets a registration error naming it; drop the define,
or use a name of the mod's own.
