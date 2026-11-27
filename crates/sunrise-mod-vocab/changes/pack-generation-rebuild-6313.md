`ShaderPackState::generation` continues its count across a renderer rebuild, on the same device
or after device-loss recovery, instead of restarting at 0 on the new renderer. The count wraps
around after `u32::MAX`.
