//! Retained mod GPU resources across the real staged pack apply and a new device's adoption.
use super::cloud_pack_fixture::{INNER, write_pack};
use crate::game::ClientGame;
use crate::game::model_baking_completed_tests::reload;
use crate::game::native_entity_refs::tests::{test_window, with_client_game};
use mod_api::__private::game::Fields;
use mod_api::content::crash::ModCall;
use mod_api::{
    BufferError, ClientHost, ComputePassDescriptor, FrameGraphSetupEvent, FramePassDescriptor,
    FrameTarget, Hooks, MeshBufferInit, MeshDescriptor, MeshError, ModEntry, ReadbackTicket,
    RenderTargetDescriptor, RenderTargetError, RenderTargetFormat, RenderTargetRef,
    StorageBufferDescriptor, StorageBufferRef,
};
use mod_platform::{content::crash::enter_native_mod, testing::with_mods};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sunrise_client::{gfx::Opened, graphics::GraphicsApi};
use sunrise_gameplay_common::game::{Screen, test_fixtures::run_fresh_cases_with_env};

const OWNER: &str = "retained_gpu_probe";
const CASE: &str = "SUNRISE_RETAINED_MOD_GPU_CASE";
const FIRST: [u8; 4] = [17, 31, 63, 127];
const QUEUED: [u8; 4] = [91, 62, 33, 4];

struct CopyAndClear {
    buffer: StorageBufferRef,
    len: u64,
    clear: Option<(RenderTargetRef, [f32; 4])>,
    ticket: Arc<Mutex<Option<ReadbackTicket>>>,
}

#[mod_api::__subscriptions]
impl Hooks for CopyAndClear {
    fn frame_graph_setup(&self, host: &dyn ClientHost, event: &mut FrameGraphSetupEvent<'_>) {
        if let Some((target, color)) = self.clear {
            event
                .graph
                .add_pass(
                    FramePassDescriptor::new("retained target color", FrameTarget::Mod(target))
                        .clear_color_0(color),
                    &mut |_| {},
                )
                .unwrap();
        }
        event
            .graph
            .add_compute_pass(
                ComputePassDescriptor::new("retained buffer copy"),
                &mut |_| {
                    *self.ticket.lock().unwrap() =
                        Some(self.buffer.request_readback(host, 0, self.len).unwrap());
                },
            )
            .unwrap();
    }
}

/// Use the actual native frame graph recorder and renderer submission without installing any
/// process-global mod or shader registration. The compute node needs only the encoder copy.
fn submit_frame(
    game: &mut ClientGame<'_>,
    buffer: StorageBufferRef,
    len: u64,
    clear: Option<(RenderTargetRef, [f32; 4])>,
) -> ReadbackTicket {
    let ticket = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        OWNER,
        Box::new(CopyAndClear {
            buffer,
            len,
            clear,
            ticket: ticket.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        assert!(game.render(Duration::ZERO).unwrap());
        let panics = mod_api::__private::take_hook_panics();
        assert!(panics.is_empty(), "{panics:?}");
    });
    ticket.lock().unwrap().take().expect("readback requested")
}

fn read_ticket(game: &mut ClientGame<'_>, ticket: ReadbackTicket) -> Vec<u8> {
    let renderer = game.client.renderer.0.as_mut().unwrap();
    renderer.wait_submissions();
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .unwrap();
    game.readback_poll(ticket)
        .unwrap()
        .expect("GPU copy mapped")
}

fn assert_pixel(game: &ClientGame<'_>, target: RenderTargetRef, expected: [u8; 4]) {
    let (_, view, _) = game
        .mod_gpu()
        .unwrap()
        .resolve_mod_render_target_color(mod_api::content::crash::current_native_mod(), target, 0)
        .unwrap();
    let renderer = game.client.renderer.0.as_ref().unwrap();
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("retained target pixel"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        view.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    renderer
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .unwrap();
    assert_eq!(&slice.get_mapped_range().unwrap()[..], &expected);
}

fn same_device(old: &wgpu::Device, current: &wgpu::Device) -> bool {
    // Each device owns its thread-local scope stack. If the handles share a device,
    // the inner (current) scope catches the error; otherwise only the old scope can.
    let old_scope = old.push_error_scope(wgpu::ErrorFilter::Validation);
    let current_scope = current.push_error_scope(wgpu::ErrorFilter::Validation);
    // wgpu rejects empty usage before allocating a GPU buffer.
    drop(old.create_buffer(&wgpu::BufferDescriptor {
        label: Some("device identity probe"),
        size: 4,
        usage: wgpu::BufferUsages::empty(),
        mapped_at_creation: false,
    }));
    let current_error = pollster::block_on(current_scope.pop());
    let old_error = pollster::block_on(old_scope.pop());
    match (current_error, old_error) {
        (Some(wgpu::Error::Validation { .. }), None) => true,
        (None, Some(wgpu::Error::Validation { .. })) => false,
        errors => panic!("unexpected (current, old) device scope results: {errors:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn retained_mod_gpu_handles_follow_the_device_lifetime() {
    if run_fresh_cases_with_env(
        "game::client_side::packs::retained_mod_gpu_tests::retained_mod_gpu_handles_follow_the_device_lifetime",
        CASE,
        &["reuse", "replace"],
        1,
        &[("WGPU_BACKEND", "vulkan")],
    ) {
        return;
    }
    with_client_game(|game| {
        let (_event_loop, window) = test_window([160, 120]);
        game.common.options.offscreen = true;
        game.client.settings.sunrise.api = GraphicsApi::Vulkan;
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        let renderer = game.new_renderer(window.clone()).unwrap();
        assert_eq!(renderer.info.backend, wgpu::Backend::Vulkan);
        game.set_renderer(renderer);
        game.client.window.0 = Some(window.clone());
        let device = game.client.renderer.0.as_ref().unwrap().device.clone();
        let _owner = enter_native_mod(ModCall {
            id: OWNER,
            hook: "retained GPU lifetime fixture",
            part: None,
        });
        let buffer_desc = StorageBufferDescriptor::new(8).copy_src();
        let target_desc =
            RenderTargetDescriptor::new([1, 1]).color_target(RenderTargetFormat::Rgba8Unorm);
        let buffer = game.create_storage_buffer(&buffer_desc).unwrap();
        let target = game.create_render_target(&target_desc).unwrap();
        game.storage_buffer_write_range(buffer, 0, &FIRST).unwrap();
        let old_ticket = submit_frame(game, buffer, 4, Some((target, [1.0, 0.0, 1.0, 1.0])));
        let mesh_buffers = [MeshBufferInit::from_bytes(24, &[0; 96])];
        let mesh_desc = MeshDescriptor::new(&mesh_buffers);
        let mesh = game.create_mesh(&mesh_desc).unwrap();
        let uniform = game.create_uniform_buffer(&[0; 16]).unwrap();

        let case = std::env::var(CASE).unwrap();
        if case == "reuse" {
            game.storage_buffer_write_range(buffer, 4, &QUEUED).unwrap();
            let generation = game.client.client_pack.render.generation;
            write_pack("retained_gpu", INNER);
            game.client.settings.resource_packs = vec!["file/retained_gpu".into()];
            reload(game);
            assert_eq!(game.client.settings.resource_packs, ["file/retained_gpu"]);
            assert_eq!(
                (
                    game.client.client_pack.clouds.0,
                    game.client.client_pack.clouds.1,
                ),
                (4, 6)
            );
            assert!(game.client.client_pack.render.generation > generation);
            assert!(
                same_device(&device, &game.client.renderer.0.as_ref().unwrap().device),
                "pack reload replaced the device"
            );

            let bytes = submit_frame(game, buffer, 8, None);
            assert_eq!(read_ticket(game, old_ticket), FIRST);
            assert_pixel(game, target, [255, 0, 255, 255]);
            assert_eq!(game.mesh_release(mesh), Err(MeshError::Stale));
            assert_eq!(
                game.uniform_buffer_release(uniform),
                Err(BufferError::Stale)
            );
            {
                let _foreign = enter_native_mod(ModCall {
                    id: "other_mod",
                    hook: "foreign GPU ref fixture",
                    part: None,
                });
                assert_eq!(game.storage_buffer_size(buffer), Err(BufferError::NotOwned));
                assert_eq!(
                    game.render_target_size(target),
                    Err(RenderTargetError::NotOwned)
                );
                assert_eq!(game.readback_poll(bytes), Err(BufferError::NotOwned));
            }
            assert_eq!(read_ticket(game, bytes), [FIRST, QUEUED].concat());
        } else {
            // The same offscreen window and Game actually receive a different wgpu device.
            // Recovery drops the old renderer before opening the new device, but the Game's
            // old facade remains reachable up to the common publication seam.
            game.client.renderer.0.as_mut().unwrap().wait_submissions();
            drop(game.client.renderer.0.take());
            let opened = pollster::block_on(Opened::for_window(
                &window,
                true,
                game.client.settings.sunrise.api,
                game.client.client_pack.render.texture_layers,
            ))
            .unwrap();
            let replacement = game
                .renderer_for(
                    window,
                    &game.common.pack,
                    &game.client.client_pack,
                    &game.client.gui,
                    Some(opened),
                )
                .unwrap();
            assert_eq!(replacement.info.backend, wgpu::Backend::Vulkan);
            game.adopt_renderer(replacement);
            assert!(
                !same_device(&device, &game.client.renderer.0.as_ref().unwrap().device),
                "renderer adoption kept the old device"
            );

            // Repopulate the first slots with the same owner and issue the replacement device's
            // first ticket *before* probing old handles; an empty store cannot catch aliasing.
            let fresh_buffer = game.create_storage_buffer(&buffer_desc).unwrap();
            let fresh_target = game.create_render_target(&target_desc).unwrap();
            game.storage_buffer_write_range(fresh_buffer, 0, &QUEUED)
                .unwrap();
            let fresh_ticket = submit_frame(
                game,
                fresh_buffer,
                4,
                Some((fresh_target, [0.0, 1.0, 0.0, 1.0])),
            );
            assert_eq!(game.storage_buffer_size(buffer), Err(BufferError::Stale));
            assert_eq!(
                game.render_target_size(target),
                Err(RenderTargetError::Stale)
            );
            assert_eq!(
                game.readback_poll(old_ticket),
                Err(BufferError::Unavailable)
            );
            assert_eq!(read_ticket(game, fresh_ticket), QUEUED);
            assert_pixel(game, fresh_target, [0, 255, 0, 255]);
        }

        // A retired ref must stay stale even when the same owner occupies its old slot.
        let fresh_mesh = game.create_mesh(&mesh_desc).unwrap();
        let fresh_uniform = game.create_uniform_buffer(&[0; 16]).unwrap();
        assert_eq!(fresh_mesh.fields().slot, mesh.fields().slot);
        assert_eq!(fresh_uniform.fields().slot, uniform.fields().slot);
        game.mesh_write_vertices(fresh_mesh, 0, 0, &QUEUED).unwrap();
        game.uniform_buffer_write(fresh_uniform, 0, &QUEUED)
            .unwrap();
        assert_eq!(
            game.mesh_write_vertices(mesh, 0, 0, &FIRST),
            Err(MeshError::Stale),
            "{case}: retired mesh must reject writes after same-owner slot reuse"
        );
        assert_eq!(
            game.uniform_buffer_write(uniform, 0, &FIRST),
            Err(BufferError::Stale),
            "{case}: retired uniform buffer must reject writes after same-owner slot reuse"
        );
        drop(game.client.renderer.0.take());
        drop(game.client.mod_gpu.take());
    });
}
