//! The wgpu twin of the OpenGL texture (Windows: windows draw with femtovg on
//! wgpu, Direct3D 12).
//!
//! One `Rgba8Unorm` texture per window, kept while the picture's size stays,
//! shown as a `slint::Image` made from the `wgpu::Texture`. Slint imports only
//! RGBA textures and wgpu has no 3-byte format, while software rendering
//! hands over RGB rows (`Rgb8Pixel`, as mpv renders them). So the bytes go
//! as they are into a reused storage buffer (`Queue::write_buffer`, the one
//! copy the OpenGL path also makes), and a full-screen triangle expands them
//! into the texture on the GPU, right before Slint's draw on the same queue.
//! Safe code: no `unsafe` here.

use super::Size;
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

/// Pixel `(x, y)` of the picture is bytes `3 * (y * width + x)` .. `+ 3` of
/// `pixels` (red, green, blue), read as little-endian words.
const SHADER: &str = r#"
struct Params { width: u32, height: u32, pad0: u32, pad1: u32 };
@group(0) @binding(0) var<storage, read> pixels: array<u32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn byte_at(offset: u32) -> u32 {
    return (pixels[offset >> 2u] >> ((offset & 3u) * 8u)) & 0xffu;
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let offset = (u32(position.y) * params.width + u32(position.x)) * 3u;
    let rgb = vec3<u32>(byte_at(offset), byte_at(offset + 1u), byte_at(offset + 2u));
    return vec4<f32>(vec3<f32>(rgb) / 255.0, 1.0);
}
"#;

/// The texture of one picture size and what fills it.
struct Target {
    size: Size,
    /// Read back by the test (the image holds its own reference).
    #[cfg_attr(not(test), allow(dead_code))]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pixels: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// One window's texture (UI thread, made at the window's rendering setup
/// with the device and queue Slint draws it with).
pub struct View {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    target: Option<Target>,
}

impl View {
    /// `None` if the device refuses the pipeline (then the host sets the
    /// pixels as images).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vestra rgb to rgba"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vestra rgb to rgba"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(1, wgpu::BufferBindingType::Uniform),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vestra rgb to rgba"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vestra rgb to rgba"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vestra rgb to rgba"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if pollster::block_on(scope.pop()).is_some() {
            return None;
        }
        let (device, queue) = (device.clone(), queue.clone());
        Some(Self {
            device,
            queue,
            pipeline,
            layout,
            params,
            target: None,
        })
    }

    pub fn size(&self) -> Size {
        self.target.as_ref().map_or((0, 0), |target| target.size)
    }

    /// Upload `buffer` and expand it into the texture (a new one when the
    /// size changed); the new texture's image when there is one.
    pub fn upload(&mut self, buffer: &SharedPixelBuffer<Rgb8Pixel>) -> Option<Image> {
        let size = (buffer.width(), buffer.height());
        let bytes = buffer.as_bytes();
        let mut fresh = None;
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.size != size)
        {
            self.target = None;
            let (target, image) = self.make(size, bytes.len() as u64)?;
            self.target = Some(target);
            fresh = Some(image);
        }
        let target = self.target.as_ref()?;
        // Words of whole bytes, then the last 1-3 bytes padded to a word
        // (buffer writes are in multiples of 4).
        let whole = bytes.len() & !3;
        if whole > 0 {
            self.queue.write_buffer(&target.pixels, 0, &bytes[..whole]);
        }
        if whole < bytes.len() {
            let mut tail = [0u8; 4];
            tail[..bytes.len() - whole].copy_from_slice(&bytes[whole..]);
            self.queue.write_buffer(&target.pixels, whole as u64, &tail);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vestra rgb to rgba"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &target.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // Submitted now: Slint's draw, submitted after this on the same
        // queue, samples the finished texture.
        self.queue.submit([encoder.finish()]);
        fresh
    }

    /// A texture of `size` and a buffer for `len` bytes; `None` when the
    /// device's limits do not allow them (the host then sets images).
    fn make(&self, (width, height): Size, len: u64) -> Option<(Target, Image)> {
        let limits = self.device.limits();
        let words = len.div_ceil(4) * 4;
        if width == 0
            || height == 0
            || width.max(height) > limits.max_texture_dimension_2d
            || words > limits.max_storage_buffer_binding_size
            || words > limits.max_buffer_size
        {
            return None;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vestra video picture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // Slint's import needs the first two; the test reads it back.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let pixels = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vestra video picture"),
            size: words,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = [width, height, 0, 0].map(u32::to_le_bytes).concat();
        self.queue.write_buffer(&self.params, 0, &params);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vestra video picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        let view = texture.create_view(&Default::default());
        let image = Image::try_from(texture.clone()).ok()?;
        Some((
            Target {
                size: (width, height),
                texture,
                view,
                pixels,
                bind,
            },
            image,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WARP (Direct3D 12's software rasterizer): no GPU needed.
    fn warp() -> (wgpu::Instance, wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .expect("WARP");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("device");
        (instance, device, queue)
    }

    fn read(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
        let size = texture.size();
        let padded = (size.width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(padded * size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        (0..size.height as usize)
            .flat_map(|row| {
                let start = row * padded as usize;
                data[start..start + size.width as usize * 4].to_vec()
            })
            .collect()
    }

    /// Odd sizes (rows of 15 and 21 bytes, a picture of 45 and 147 bytes:
    /// not whole words) come out pixel for pixel, top row first, opaque.
    #[test]
    fn warp_rgb_pictures_become_the_same_rgba_texture_reused_per_size() {
        let (_instance, device, queue) = warp();
        let mut view = View::new(&device, &queue).expect("pipeline");
        for (round, (width, height)) in [(5u32, 3u32), (5, 3), (7, 7)].into_iter().enumerate() {
            let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(width, height);
            for (index, pixel) in buffer.make_mut_slice().iter_mut().enumerate() {
                let index = index as u8;
                *pixel = Rgb8Pixel::new(index * 3, 255 - index, index ^ 0x5a ^ round as u8);
            }
            let image = view.upload(&buffer);
            assert_eq!(
                image.is_some(),
                round != 1,
                "a new texture only for a new size"
            );
            assert_eq!(view.size(), (width, height));
            let texture = &view.target.as_ref().unwrap().texture;
            let rgba = read(&device, &queue, texture);
            let expected: Vec<u8> = buffer
                .as_slice()
                .iter()
                .flat_map(|p| [p.r, p.g, p.b, 255])
                .collect();
            assert_eq!(rgba, expected, "{width}x{height}");
        }
    }
}
