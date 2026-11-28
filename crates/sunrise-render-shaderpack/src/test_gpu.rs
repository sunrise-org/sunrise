//! The skip policy shared by GPU fixtures, and this crate's adapter and device helpers.

/// Report why a GPU test cannot run. Fail instead when
/// `SUNRISE_GPU_TESTS_REQUIRE_ADAPTER=1`.
#[track_caller]
pub fn skip_or_fail(reason: impl std::fmt::Display) {
    let required =
        std::env::var_os("SUNRISE_GPU_TESTS_REQUIRE_ADAPTER").is_some_and(|value| value == "1");
    skip_or_fail_with(required, reason);
}

/// The same policy with the switch supplied explicitly, for tests that cannot change the environment.
#[track_caller]
pub fn skip_or_fail_with(required: bool, reason: impl std::fmt::Display) {
    assert!(
        !required,
        "GPU test could not run: {reason} (SUNRISE_GPU_TESTS_REQUIRE_ADAPTER=1)"
    );
    eprintln!("skipped: {reason}");
}

#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

#[cfg(test)]
static GPU_TEST: Mutex<()> = Mutex::new(());

/// Serialize an entire GPU test, including driver initialization and teardown.
#[cfg(test)]
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    GPU_TEST.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12, not OpenGL:
/// WGL instances made and dropped from many test threads at once deadlocked NVIDIA's driver,
/// and the render tests hung most runs (#5238).
#[cfg(test)]
fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}

/// Find an adapter, reporting an unavailable adapter without hiding other failures.
#[cfg(test)]
pub(crate) fn adapter() -> Option<wgpu::Adapter> {
    match pollster::block_on(instance().request_adapter(&Default::default())) {
        Ok(adapter) => Some(adapter),
        Err(error @ wgpu::RequestAdapterError::NotFound { .. }) => {
            skip_or_fail(format_args!("no GPU adapter: {error}"));
            None
        }
        Err(error) => panic!("GPU test adapter request failed: {error}"),
    }
}

/// Device creation may skip locally, but must succeed under the CI switch.
#[cfg(test)]
pub(crate) fn optional_device_with(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    match pollster::block_on(adapter.request_device(descriptor)) {
        Ok(pair) => Some(pair),
        Err(error) => {
            skip_or_fail(format_args!("GPU test device creation failed: {error}"));
            None
        }
    }
}

#[cfg(test)]
pub(crate) fn device(adapter: &wgpu::Adapter) -> (wgpu::Device, wgpu::Queue) {
    pollster::block_on(adapter.request_device(&Default::default()))
        .expect("GPU test device creation failed")
}
