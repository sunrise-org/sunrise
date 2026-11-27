Meshes and uniform buffers still retire on renderer rebuild, even when the device is reused.
A later allocation by the same mod can no longer make the retired `MeshRef` or `UniformBufferRef`
valid again.
