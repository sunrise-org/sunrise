# Built-in far terrain removed

The engine no longer draws terrain beyond the rendered chunks, samples or serves it, or reads BlueMap tiles. The `sunrise:lod` channel is no longer reserved, no engine code listens on it, and the network thread no longer swallows its packets, so `client_packet` hears any that arrive. The `lodDistance`, `farDetail` and `blueMapRadius` client settings, the `lodZoomDetail` extra and the dedicated `sunrise-lod-distance` property are no longer read.

Hooks, render pipelines, viewed levels and the detached chunk generator are unchanged.
