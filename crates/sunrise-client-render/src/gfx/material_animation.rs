//! Animated LabPBR maps in the sprite arrays (`Renderer::animate`): a map's frame follows its
//! colour's (`SpriteAnimation::material_frame`), and goes into the arrays only when that frame
//! changes, with every level a box mean of the one above, as the load made the arrays'
//! (`prepared_textures::box_mip_chain`).
use super::{prepared_textures::box_mip_chain, upload::Uploads};
use crate::sprites::SpriteAnimation;

/// The sprite animations with a LabPBR map, and the frame of each map the arrays hold.
pub(crate) struct MaterialAnimations {
    animations: Vec<SpriteAnimation>,
    /// Per animation, the frame of its normal and specular map in the arrays.
    shown: Vec<[usize; 2]>,
}

/// A map frame the arrays don't hold yet: its layer, its kind (0 normals, 1 specular) and its
/// mip chain at the arrays' size, every level after the one above.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MaterialFrame {
    pub(crate) layer: u32,
    pub(crate) kind: usize,
    pub(crate) chain: Vec<u8>,
}

impl MaterialAnimations {
    /// The animations of `animations` with a map, each holding the frame the load put in the
    /// arrays: the one its colour shows at tick 0 (`Sprites::finish`).
    pub(crate) fn new(animations: &[SpriteAnimation]) -> Self {
        let animations: Vec<SpriteAnimation> = animations
            .iter()
            .filter(|a| a.has_materials())
            .cloned()
            .collect();
        let shown = animations
            .iter()
            .map(|a| [0, 1].map(|kind| a.material_frame(kind, 0).unwrap_or(0)))
            .collect();
        Self { animations, shown }
    }

    /// The maps of the `changed` layers whose frame at `tick` the arrays don't hold, with their
    /// chains at `size`; from now on the arrays hold them. A map of a single frame never changes.
    pub(crate) fn due(&mut self, tick: u64, changed: &[u32], size: u32) -> Vec<MaterialFrame> {
        let mut due = Vec::new();
        for (animation, shown) in self.animations.iter().zip(&mut self.shown) {
            if !changed.contains(&animation.layer) {
                continue;
            }
            for (kind, shown) in shown.iter_mut().enumerate() {
                let Some(frame) = animation.material_frame(kind, tick) else {
                    continue;
                };
                if frame == *shown {
                    continue;
                }
                *shown = frame;
                let Some(pixels) = animation.material_pixels(kind, tick) else {
                    continue;
                };
                let from = (pixels.len() / 4).isqrt() as u32;
                let top = crate::sprites::resample(pixels, from, size);
                due.push(MaterialFrame {
                    layer: animation.layer,
                    kind,
                    chain: box_mip_chain(&top, size, 1),
                });
            }
        }
        due
    }

    /// Uploads the maps `due` finds into `arrays` (each a normals and a specular array of
    /// `split` layers), every level they have; the layers it wrote, by kind.
    pub(crate) fn upload(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut Uploads,
        arrays: &[[wgpu::Texture; 2]],
        split: u32,
        tick: u64,
        changed: &[u32],
    ) -> [Vec<u32>; 2] {
        let mut written = [Vec::new(), Vec::new()];
        let Some(size) = arrays.first().map(|pair| pair[0].width()) else {
            return written;
        };
        for frame in self.due(tick, changed, size) {
            let Some(pair) = arrays.get((frame.layer / split) as usize) else {
                continue;
            };
            let texture = &pair[frame.kind];
            let mut at = 0;
            for level in 0..texture.mip_level_count() {
                let side = (texture.width() >> level).max(1);
                let bytes = (side * side * 4) as usize;
                let Some(pixels) = frame.chain.get(at..at + bytes) else {
                    break;
                };
                at += bytes;
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: frame.layer % split,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    side,
                    side,
                    pixels,
                );
            }
            written[frame.kind].push(frame.layer);
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 map frame whose four texels differ in every channel.
    fn frame(base: u8) -> Vec<u8> {
        [
            [0u8, 40, 80, 120],
            [100, 4, 8, 12],
            [200, 60, 16, 24],
            [40, 20, 120, 200],
        ]
        .iter()
        .flat_map(|t| t.map(|c| c + base))
        .collect()
    }

    /// Two animations on layers 3 and 5, a frame a tick: layer 3's specular map has a frame per
    /// colour frame and its normal map one frame; layer 5's normal map has two.
    fn animations() -> MaterialAnimations {
        let mut three = crate::sprites::test_animation(2, 3);
        three.set_materials([Some(vec![frame(1)]), Some(vec![frame(0), frame(2)])]);
        let mut five = crate::sprites::test_animation(2, 5);
        five.set_materials([Some(vec![frame(3), frame(5)]), None]);
        MaterialAnimations::new(&[three, crate::sprites::test_animation(2, 4), five])
    }

    #[test]
    fn a_changed_map_frame_uploads_box_means_once() {
        let mut maps = animations();
        let due = maps.due(1, &[3, 4, 5], 2);
        // Level 0 is the frame; level 1 each channel's mean, rounded: (a + b + c + d + 2) / 4.
        let mut specular = frame(2);
        specular.extend([87, 33, 58, 91]);
        let mut normals = frame(5);
        normals.extend([90, 36, 61, 94]);
        assert_eq!(
            due,
            [
                MaterialFrame {
                    layer: 3,
                    kind: 1,
                    chain: specular
                },
                MaterialFrame {
                    layer: 5,
                    kind: 0,
                    chain: normals
                },
            ]
        );
        // The same frames again: nothing new for the arrays.
        assert_eq!(maps.due(1, &[3, 4, 5], 2), []);
        // Back to the first frames, which the arrays held before.
        let back = maps.due(2, &[3, 5], 2);
        assert_eq!(
            back.iter().map(|f| (f.layer, f.kind)).collect::<Vec<_>>(),
            [(3, 1), (5, 0)]
        );
    }

    #[test]
    fn a_static_map_or_a_layer_that_did_not_change_uploads_nothing() {
        let mut maps = animations();
        // Only layer 3's colour changed: its specular frame is due, its normal map (one frame)
        // never is, and layer 5 waits.
        let due = maps.due(1, &[3], 2);
        assert_eq!(
            due.iter().map(|f| (f.layer, f.kind)).collect::<Vec<_>>(),
            [(3, 1)]
        );
        // The arrays still hold layer 5's frame 0, so at tick 3 (frame 1) its normal map is due.
        let due = maps.due(3, &[5], 2);
        assert_eq!(
            due.iter().map(|f| (f.layer, f.kind)).collect::<Vec<_>>(),
            [(5, 0)]
        );
        // Tick 0 shows the frames the load put in the arrays.
        assert_eq!(animations().due(0, &[3, 4, 5], 2), []);
    }

    #[test]
    fn a_map_smaller_than_the_arrays_is_scaled_by_whole_texels_first() {
        let mut maps = animations();
        let due = maps.due(1, &[3], 4);
        // Each texel of the 2x2 frame becomes a 2x2 block of level 0, so level 1 is the frame.
        let chain = &due[0].chain;
        assert_eq!(chain.len(), (16 + 4 + 1) * 4);
        assert_eq!(&chain[64..80], frame(2).as_slice());
        assert_eq!(&chain[80..], [87, 33, 58, 91]);
    }

    #[test]
    #[ignore = "daily"]
    fn uploads_write_every_level_of_the_due_layers() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((adapter, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        eprintln!("adapter {:?}", adapter.get_info());
        let array = |label| {
            wgpu::util::DeviceExt::create_texture_with_data(
                &device,
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: 2,
                        height: 2,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 2,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[7; 6 * (16 + 4)],
            )
        };
        let arrays = [[array("normals"), array("specular")]];
        let mut uploads = Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        let written = animations().upload(&mut encoder, &mut uploads, &arrays, 6, 1, &[3, 4, 5]);
        assert_eq!(written, [vec![5], vec![3]]);
        // Level 1 of every layer, normals then specular, read back one row each.
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 2 * 6 * 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for (kind, pair) in arrays[0].iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: pair,
                    mip_level: 1,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: (kind * 6 * 256) as u64,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
            );
        }
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range().unwrap();
        let texel = |kind: usize, layer: usize| {
            let at = (kind * 6 + layer) * 256;
            <[u8; 4]>::try_from(&data[at..at + 4]).unwrap()
        };
        for kind in 0..2 {
            for layer in 0..6 {
                let want = match (kind, layer) {
                    (0, 5) => [90, 36, 61, 94],
                    (1, 3) => [87, 33, 58, 91],
                    _ => [7; 4],
                };
                assert_eq!(texel(kind, layer), want, "kind {kind}, layer {layer}");
            }
        }
    }
}
