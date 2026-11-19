//! Draws a meshed world with wgpu, into a window's surface or an offscreen image.

use crate::bake::BlockModels;
use crate::mesh::{Mesh, Vertex};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

/// A first-person camera, in Minecraft's conventions: yaw 0 looks south (+Z) and turns
/// toward west as it grows; positive pitch looks down. Angles are in degrees.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Vertical field of view, as vanilla's FOV setting (70 by default).
    pub fov: f32,
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        Vec3::new(
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        )
    }

    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        // wgpu's clip space is y-up with depth 0..1, which glam calls the DirectX convention.
        glam::camera::rh::proj::directx::perspective(self.fov.to_radians(), aspect, 0.05, far)
            * glam::camera::rh::view::look_to_mat4(self.eye, self.forward(), Vec3::Y)
    }
}

/// Render settings that aren't the camera.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Render distance in blocks; fog ends here.
    pub distance: f32,
    /// Game ticks since the world started, which animate end portals.
    pub ticks: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    fog_color: [f32; 4],
    sky_color: [f32; 4],
    params: [f32; 4],
    time: [f32; 4],
}

/// Vanilla's plains sky and fog colours at noon.
const SKY: u32 = 0x78A7FF;
const FOG: u32 = 0xC0D8FF;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn color(c: u32) -> [f32; 4] {
    [
        (c >> 16 & 0xFF) as f32 / 255.0,
        (c >> 8 & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    portal: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    portal_bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    indices: Option<wgpu::Buffer>,
    /// Opaque, cutout, translucent and portal vertices, with their index counts.
    layers: [Option<(wgpu::Buffer, u32)>; 4],
    depth: Option<(wgpu::TextureView, u32, u32)>,
}

impl Renderer {
    /// A device with no window, for screenshots.
    pub fn headless(models: &BlockModels) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .context("no GPU adapter (install a Vulkan driver, such as Mesa's lavapipe)")?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        Ok(Self::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
            models,
        ))
    }

    /// Vanilla draws into a plain (not sRGB) framebuffer, so `format` should be one too
    /// for colours to match.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        models: &BlockModels,
    ) -> Self {
        let atlas = &models.atlas;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("atlas"),
                size: wgpu::Extent3d {
                    width: atlas.width,
                    height: atlas.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: atlas.levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &atlas.levels.concat(),
        );
        // Nearest within a mipmap level (block textures are pixel art), blended between
        // levels, as vanilla samples its block atlas.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        // The portal shader's textures repeat across the screen, unfiltered as vanilla
        // samples them.
        let portal_texture = |label: &str, img: &image::RgbaImage| {
            device
                .create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: img.width(),
                            height: img.height(),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    img.as_raw(),
                )
                .create_view(&Default::default())
        };
        let portal_textures = &models.portal_textures;
        let portal_sky = portal_texture("end sky", &portal_textures.sky);
        let portal_stars = portal_texture("end portal", &portal_textures.stars);
        let repeat = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let portal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("portal"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let portal_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("portal"),
            layout: &portal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&portal_sky),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&portal_stars),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&repeat),
                },
            ],
        });
        let portal_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("portal"),
                bind_group_layouts: &[Some(&layout), Some(&portal_layout)],
                ..Default::default()
            });

        let vertex_layout = Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4, 3 => Unorm8x4],
        });
        let pipeline =
            |fragment: &str, blend: Option<wgpu::BlendState>, depth_write: bool, sky: bool| {
                let portal = fragment == "fs_portal";
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(fragment),
                    layout: Some(if portal {
                        &portal_pipeline_layout
                    } else {
                        &pipeline_layout
                    }),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(match (sky, portal) {
                            (true, _) => "vs_sky",
                            (_, true) => "vs_portal",
                            _ => "vs",
                        }),
                        buffers: if sky {
                            &[]
                        } else {
                            std::slice::from_ref(&vertex_layout)
                        },
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: if sky { None } else { Some(wgpu::Face::Back) },
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(depth_write),
                        depth_compare: Some(if sky {
                            wgpu::CompareFunction::Always
                        } else {
                            wgpu::CompareFunction::LessEqual
                        }),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let sky = pipeline("fs_sky", None, false, true);
        let opaque = pipeline("fs_opaque", None, true, false);
        let cutout = pipeline("fs_cutout", None, true, false);
        let translucent = pipeline(
            "fs_translucent",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            false,
        );
        let portal = pipeline("fs_portal", None, true, false);
        Self {
            device,
            queue,
            sky,
            opaque,
            cutout,
            translucent,
            portal,
            bind_group,
            portal_bind_group,
            uniforms,
            indices: None,
            layers: [None, None, None, None],
            depth: None,
        }
    }

    /// Replaces what's drawn with `mesh`.
    pub fn upload(&mut self, mesh: &Mesh) {
        let lists = [&mesh.opaque, &mesh.cutout, &mesh.translucent, &mesh.portal];
        let quads = lists.iter().map(|l| l.len() / 4).max().unwrap_or(0);
        let indices: Vec<u32> = (0..quads as u32)
            .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
            .collect();
        self.indices = Some(
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("indices"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
        );
        for (slot, vertices) in self.layers.iter_mut().zip(lists) {
            *slot = (!vertices.is_empty()).then(|| {
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                (buffer, (vertices.len() / 4 * 6) as u32)
            });
        }
    }

    /// Draws a frame into `target`, which is `width` × `height`.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        camera: &Camera,
        view: &View,
    ) {
        if self
            .depth
            .as_ref()
            .is_none_or(|(_, w, h)| (*w, *h) != (width, height))
        {
            let depth = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some((depth.create_view(&Default::default()), width, height));
        }
        let view_proj = camera.view_proj(width as f32 / height as f32, view.distance * 2.0);
        // Vanilla's terrain fog: it ends at the render distance and starts a tenth before.
        let fog_end = view.distance;
        let fog_start = fog_end - (fog_end / 10.0).clamp(4.0, 64.0);
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: camera.eye.extend(1.0).into(),
            fog_color: color(FOG),
            sky_color: color(SKY),
            params: [fog_start, fog_end, 1.0, 0.5],
            // `GlobalSettingsUniform`'s GameTime.
            time: [(view.ticks % 24000.0) / 24000.0, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().expect("made above").0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
            if let Some(indices) = &self.indices {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                // Portals are opaque, so they draw before the translucent layer.
                let order = [
                    (&self.opaque, 0),
                    (&self.cutout, 1),
                    (&self.portal, 3),
                    (&self.translucent, 2),
                ];
                pass.set_bind_group(1, &self.portal_bind_group, &[]);
                for (pipeline, slot) in order {
                    if let Some((buffer, count)) = &self.layers[slot] {
                        pass.set_pipeline(pipeline);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw_indexed(0..*count, 0, 0..1);
                    }
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Draws a frame offscreen and reads it back. Needs a renderer made by [`Renderer::headless`].
    pub fn screenshot(
        &mut self,
        width: u32,
        height: u32,
        camera: &Camera,
        view: &View,
    ) -> Result<image::RgbaImage> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.render(
            &texture.create_view(&Default::default()),
            width,
            height,
            camera,
            view,
        );

        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("mapping the screenshot"));
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        let data = buffer.slice(..).get_mapped_range()?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
        }
        Ok(image::RgbaImage::from_raw(width, height, pixels).expect("size matches"))
    }
}
