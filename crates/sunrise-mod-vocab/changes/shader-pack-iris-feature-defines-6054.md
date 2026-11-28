# Pack defines may not name IRIS_FEATURE_*

`PackBinding::validate_syntax` now refuses a define named `IRIS_FEATURE_*`, as it refuses `GL_` names: registration
refuses a define it used to accept. Before, registration accepted such a define and the renderer passed it to the
shader pack. Those macros are now the engine's answers to the feature flags a pack lists in `iris.features.optional`
and `iris.features.required`: the engine defines one for each flag it supports that the pack lists, and drops any
other before reading the pack, so a mod's define of such a name would no longer reach the pack. A mod that declared
one now gets a registration error naming it; drop the define, or use a name of the mod's own.
