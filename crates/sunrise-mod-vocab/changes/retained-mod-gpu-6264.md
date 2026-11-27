Storage buffers, render targets and outstanding readbacks survive same-device renderer rebuilds.
After device replacement, old storage and target handles are stale and old readback tickets are
unavailable. Recreate the resources and request new readbacks on the replacement device.
