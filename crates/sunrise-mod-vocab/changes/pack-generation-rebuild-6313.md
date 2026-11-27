`ShaderPackState::generation` continues across a renderer rebuild, on the same device or after
device-loss recovery: a pack loaded on the new renderer never repeats a generation the old one
reported.
