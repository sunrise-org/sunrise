//! The GPU tests' adapter and device helpers.
//!
//! The client render's `gfx::test_gpu` hands out the same four (`lock`, `instance`, `adapter`
//! and `device`, each spelled the same), and its own extra helpers (the DX12 compilers, the
//! depth-load probe) are its tests' alone, so this crate keeps only what it calls.

use std::sync::{Mutex, MutexGuard};

static GPU_TEST: Mutex<()> = Mutex::new(());

/// Serialize an entire GPU test, including driver initialization and teardown.
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    GPU_TEST.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12, not OpenGL:
/// WGL instances made and dropped from many test threads at once deadlocked NVIDIA's driver,
/// and the render tests hung most runs (#5238).
pub(crate) fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}

/// Find an adapter, reporting an unavailable adapter without hiding other failures.
pub(crate) fn adapter() -> Option<wgpu::Adapter> {
    let instance = instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error @ wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipped: no adapter: {error}");
            return None;
        }
        Err(error) => panic!("GPU test adapter request failed: {error}"),
    };
    Some(adapter)
}

pub(crate) fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&Default::default()))
        .expect("GPU test device creation failed")
}
