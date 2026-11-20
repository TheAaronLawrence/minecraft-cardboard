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
    /// A transform in view space between the view and the projection: the walking bob
    /// and hurt tilt (`GameRenderer.bobView` and `bobHurt`), which shake the world and the
    /// hands but not where the player looks.
    pub bob: Mat4,
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
            * self.view()
    }

    /// World space to view space, bob and all.
    pub fn view(&self) -> Mat4 {
        self.bob * glam::camera::rh::view::look_to_mat4(self.eye, self.forward(), Vec3::Y)
    }
}

/// Render settings that aren't the camera.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Render distance in blocks; fog ends here.
    pub distance: f32,
    /// Game ticks since the world started, which animate end portals.
    pub ticks: f32,
    /// Draw the crosshair (a window does; screenshots for comparison don't).
    pub hud: bool,
}

/// A corner of an outline edge's quad: the edge's end, its direction, and which side of
/// the line this corner is on. The vertex shader widens the line on screen.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineVertex {
    pos: [f32; 3],
    dir: [f32; 3],
    side: f32,
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
    /// Width and height in pixels, the outline width in pixels, and the GUI scale.
    screen: [f32; 4],
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

/// Vanilla's automatic GUI scale: the largest that leaves the screen at least 320 × 240
/// GUI pixels (`Window.calculateScale` with the setting at 0).
pub fn gui_scale(width: u32, height: u32) -> u32 {
    let mut scale = 1;
    while scale < width
        && scale < height
        && width / (scale + 1) >= 320
        && height / (scale + 1) >= 240
    {
        scale += 1;
    }
    scale
}

/// A chunk's vertex buffer and index count per layer, if it has any quads in it.
type ChunkBuffers = [Option<(wgpu::Buffer, u32)>; 4];

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The block and entity atlas, and its full size, for writing sprites into it later.
    atlas: wgpu::Texture,
    atlas_size: [u32; 2],
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    entity: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    see_through: wgpu::RenderPipeline,
    portal: wgpu::RenderPipeline,
    outline: wgpu::RenderPipeline,
    crosshair: wgpu::RenderPipeline,
    gui: wgpu::RenderPipeline,
    crack: wgpu::RenderPipeline,
    hud_art: crate::hud::HudArt,
    hotbar: Option<crate::hud::Hotbar>,
    /// A screen drawn instead of the HUD: the game's menus, or the launcher.
    screen: Option<crate::hud::Screen>,
    /// The cracks over blocks being mined, and their index count.
    crack_vertices: Option<(wgpu::Buffer, u32)>,
    /// Players, mobs and dropped items, and their index count.
    entity_vertices: Option<(wgpu::Buffer, u32)>,
    /// Entities' glowing layers, and their index count.
    eye_vertices: Option<(wgpu::Buffer, u32)>,
    /// Name tags' see-through parts, and their opaque text, with index counts.
    see_through_vertices: Option<(wgpu::Buffer, u32)>,
    tag_vertices: Option<(wgpu::Buffer, u32)>,
    /// The outline around the block the player looks at, and its index count.
    outline_vertices: Option<(wgpu::Buffer, u32)>,
    /// The player's own arm and held items, and their index count.
    hand_vertices: Option<(wgpu::Buffer, u32)>,
    /// The fire a burning player sees, and its index count.
    fire_overlay_vertices: Option<(wgpu::Buffer, u32)>,
    bind_group: wgpu::BindGroup,
    /// The same bindings, with the uniforms the hand is drawn with.
    hand_bind_group: wgpu::BindGroup,
    hand_uniforms: wgpu::Buffer,
    portal_bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    /// Six indices per quad, enough for the largest chunk's largest layer.
    indices: Option<(wgpu::Buffer, u32)>,
    /// Each chunk's opaque, cutout, translucent and portal vertices, with index counts.
    chunks: std::collections::HashMap<(i32, i32), ChunkBuffers>,
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
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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
        let hand_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand uniforms"),
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
        let atlas_view = texture.create_view(&Default::default());
        let bind_group_with = |uniforms: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&atlas_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let bind_group = bind_group_with(&uniforms);
        let hand_bind_group = bind_group_with(&hand_uniforms);
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
        let pipeline = |fragment: &str,
                        blend: Option<wgpu::BlendState>,
                        depth_write: bool,
                        sky: bool,
                        cull: bool| {
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
                    cull_mode: cull.then_some(wgpu::Face::Back),
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
        let sky = pipeline("fs_sky", None, false, true, false);
        let opaque = pipeline("fs_opaque", None, true, false, true);
        let cutout = pipeline("fs_cutout", None, true, false, true);
        // `RenderPipelines.ENTITY_CUTOUT`: entities show their boxes' back faces too, which
        // some textures paint where the front is clear, such as a chicken's legs.
        let entity = pipeline("fs_cutout", None, true, false, false);
        let translucent = pipeline(
            "fs_translucent",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            false,
            true,
        );
        let portal = pipeline("fs_portal", None, true, false, true);
        let line_layout = Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32],
        });
        // Drawn over the world, tested against its depth but not writing it.
        let overlay = |vertex: &str,
                       fragment: &str,
                       buffers: &[Option<wgpu::VertexBufferLayout>],
                       blend: wgpu::BlendState,
                       depth_compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    buffers,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let outline = overlay(
            "vs_line",
            "fs_line",
            std::slice::from_ref(&line_layout),
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::CompareFunction::LessEqual,
        );
        // Vanilla's crosshair inverts what's behind it.
        let invert = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDst,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        };
        let crosshair = overlay(
            "vs_sky",
            "fs_crosshair",
            &[],
            wgpu::BlendState {
                color: invert,
                alpha: wgpu::BlendComponent::OVER,
            },
            wgpu::CompareFunction::Always,
        );
        let vertex_layouts = std::slice::from_ref(&vertex_layout);
        // `RenderPipelines.TEXT_SEE_THROUGH`: blended over whatever is in front.
        let see_through = overlay(
            "vs",
            "fs_translucent",
            vertex_layouts,
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::CompareFunction::Always,
        );
        // The HUD has a depth buffer of its own, so items' faces hide each other.
        let gui = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gui"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_gui"),
                buffers: vertex_layouts,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_gui"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // `RenderTypes.crumbling`: the crack texture doubles or darkens what's under it, drawn
        // just in front of the block's faces.
        let crack = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crack"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: vertex_layouts,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_crack"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Dst,
                            dst_factor: wgpu::BlendFactor::Src,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                // `glPolygonOffset(-1, -10)`.
                bias: wgpu::DepthBiasState {
                    constant: -10,
                    slope_scale: -1.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            atlas_size: [atlas.width, atlas.height],
            atlas: texture,
            device,
            queue,
            sky,
            opaque,
            cutout,
            entity,
            translucent,
            see_through,
            portal,
            outline,
            crosshair,
            gui,
            crack,
            hud_art: models.hud.clone(),
            hotbar: None,
            screen: None,
            crack_vertices: None,
            entity_vertices: None,
            eye_vertices: None,
            see_through_vertices: None,
            tag_vertices: None,
            outline_vertices: None,
            hand_vertices: None,
            fire_overlay_vertices: None,
            bind_group,
            hand_bind_group,
            hand_uniforms,
            portal_bind_group,
            uniforms,
            indices: None,
            chunks: Default::default(),
            depth: None,
        }
    }

    /// Draws `mesh` for the chunk at `key` (chunk x, z), replacing what it drew before.
    pub fn upload_chunk(&mut self, key: (i32, i32), mesh: &Mesh) {
        let lists = [&mesh.opaque, &mesh.cutout, &mesh.translucent, &mesh.portal];
        let quads = lists.iter().map(|l| l.len() / 4).max().unwrap_or(0) as u32;
        self.ensure_indices(quads);
        let buffers = lists.map(|vertices| {
            (!vertices.is_empty()).then(|| {
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::cast_slice(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                (buffer, (vertices.len() / 4 * 6) as u32)
            })
        });
        self.chunks.insert(key, buffers);
    }

    /// Outlines these edges (world coordinates, from one end to the other), as vanilla
    /// outlines the block the player looks at; none clears it.
    pub fn set_outline(&mut self, edges: &[[f32; 6]]) {
        if edges.is_empty() {
            self.outline_vertices = None;
            return;
        }
        let vertices: Vec<LineVertex> = edges
            .iter()
            .flat_map(|e| {
                let (a, b) = (Vec3::from_slice(&e[..3]), Vec3::from_slice(&e[3..]));
                let dir = (b - a).normalize_or_zero().to_array();
                [(a, 1.0), (a, -1.0), (b, -1.0), (b, 1.0)].map(|(p, side)| LineVertex {
                    pos: p.to_array(),
                    dir,
                    side,
                })
            })
            .collect();
        self.ensure_indices(edges.len() as u32);
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("outline"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        self.outline_vertices = Some((buffer, edges.len() as u32 * 6));
    }

    /// The hotbar to draw over the world, or none.
    pub fn set_hotbar(&mut self, hotbar: Option<crate::hud::Hotbar>) {
        self.hotbar = hotbar;
    }

    /// The screen to draw over everything instead of the HUD, or none.
    pub fn set_screen(&mut self, screen: Option<crate::hud::Screen>) {
        self.screen = screen;
    }

    /// Draws these crack quads (from `hud::cracks`) over blocks being mined.
    pub fn set_cracks(&mut self, vertices: &[Vertex]) {
        self.crack_vertices = self.quads_buffer("cracks", vertices);
    }

    /// Draws these entity quads (from `entity::vertices`) after the cutout blocks.
    pub fn set_entities(&mut self, meshes: &crate::entity::Meshes) {
        self.entity_vertices = self.quads_buffer("entities", &meshes.cutout);
        self.eye_vertices = self.quads_buffer("entity eyes", &meshes.eyes);
        self.see_through_vertices = self.quads_buffer("name tags behind", &meshes.see_through);
        self.tag_vertices = self.quads_buffer("name tags", &meshes.tags);
    }

    /// Draws these quads (from `entity::first_person`), the player's own hands, over the
    /// world, as vanilla draws them after clearing the depth buffer, at a field of view of
    /// 70 whatever the view's is (`Camera.calculateHudFov`).
    pub fn set_hand(&mut self, vertices: &[Vertex]) {
        self.hand_vertices = self.quads_buffer("hand", vertices);
    }

    /// Draws these quads (from `entity::fire_overlay`) over the hands, blended and
    /// whatever their depth, as `ScreenEffectRenderer` draws the fire a burning player
    /// sees.
    pub fn set_fire_overlay(&mut self, vertices: &[Vertex]) {
        self.fire_overlay_vertices = self.quads_buffer("fire overlay", vertices);
    }

    /// Replaces a sprite's pixels in the atlas, mipmaps and all, such as a skin slot's
    /// (`entity::skin_slot`) with a downloaded skin. `img` must be the sprite's size.
    pub fn write_sprite(&self, sprite: crate::atlas::Sprite, img: &image::RgbaImage) {
        let [w, h] = self.atlas_size;
        let x = (sprite.rect[0] * w as f32).round() as u32;
        let y = (sprite.rect[1] * h as f32).round() as u32;
        let levels = self.atlas.mip_level_count() - 1;
        let mips = crate::mipmap::generate(img.clone(), crate::mipmap::Strategy::Auto, 0.0, levels);
        for (level, mip) in mips.iter().enumerate() {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: x >> level,
                        y: y >> level,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                mip.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip.width() * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: mip.width(),
                    height: mip.height(),
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// A vertex buffer of whole quads and its index count, growing the index buffer to fit.
    fn quads_buffer(&mut self, label: &str, vertices: &[Vertex]) -> Option<(wgpu::Buffer, u32)> {
        if vertices.is_empty() {
            return None;
        }
        let quads = (vertices.len() / 4) as u32;
        self.ensure_indices(quads);
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        Some((buffer, quads * 6))
    }

    /// Makes the shared index buffer hold at least `quads` quads.
    fn ensure_indices(&mut self, quads: u32) {
        if self.indices.as_ref().is_none_or(|(_, n)| *n < quads) {
            // Grow to the next power of two, so a stream of chunks rarely regrows it.
            let n = quads.next_power_of_two();
            let indices: Vec<u32> = (0..n)
                .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
                .collect();
            let buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("indices"),
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
            self.indices = Some((buffer, n));
        }
    }

    pub fn remove_chunk(&mut self, key: (i32, i32)) {
        self.chunks.remove(&key);
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
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
        // `AtmosphericFogEnvironment.getBaseColor`: the nearer the render distance, the more
        // the fog takes on the sky's colour, so short distances don't look washed out.
        let chunks = view.distance / 16.0;
        let toward_sky = 1.0 - (0.25 + 0.75 * chunks / 32.0).clamp(0.25, 1.0).powf(0.25);
        let (fog, sky) = (color(FOG), color(SKY));
        let fog_color = std::array::from_fn(|i| fog[i] + (sky[i] - fog[i]) * toward_sky);
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: camera.eye.extend(1.0).into(),
            fog_color,
            // The sky fogs out at the render distance (`FogData.skyEnd`).
            sky_color: [sky[0], sky[1], sky[2], view.distance],
            params: [fog_start, fog_end, 1.0, 0.5],
            // `GlobalSettingsUniform`'s GameTime.
            time: [(view.ticks % 24000.0) / 24000.0, 0.0, 0.0, 0.0],
            screen: [
                width as f32,
                height as f32,
                // `Window.getAppropriateLineWidth`.
                (width as f32 / 1920.0 * 2.5).max(2.5),
                gui_scale(width, height) as f32,
            ],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let hand_view_proj = Camera {
            fov: 70.0,
            ..*camera
        }
        .view_proj(width as f32 / height as f32, view.distance * 2.0);
        let hand_uniforms = Uniforms {
            view_proj: hand_view_proj.to_cols_array_2d(),
            inv_view_proj: hand_view_proj.inverse().to_cols_array_2d(),
            ..uniforms
        };
        self.queue
            .write_buffer(&self.hand_uniforms, 0, bytemuck::bytes_of(&hand_uniforms));
        let scale = gui_scale(width, height);
        let (gui_width, gui_height) = (width.div_ceil(scale), height.div_ceil(scale));
        let mut gui_mesh = match (&self.screen, &self.hotbar, view.hud) {
            (Some(crate::hud::Screen::Menu(menu)), ..) => {
                crate::hud::menu::menu(&self.hud_art, menu, gui_width, gui_height)
            }
            (Some(crate::hud::Screen::Launcher(launcher)), ..) => {
                crate::hud::launcher::launcher(&self.hud_art, launcher, gui_width, gui_height)
            }
            (
                None | Some(crate::hud::Screen::Pause(_) | crate::hud::Screen::Inventory(_)),
                Some(hotbar),
                true,
            ) => crate::hud::hotbar(
                &self.hud_art,
                hotbar,
                width.div_ceil(scale),
                height.div_ceil(scale),
            ),
            _ => Default::default(),
        };
        let (gw, gh) = (gui_width as i32, gui_height as i32);
        match &self.screen {
            Some(crate::hud::Screen::Pause(pause)) => gui_mesh.pause(&self.hud_art, pause, gw, gh),
            Some(crate::hud::Screen::Inventory(inv)) => {
                gui_mesh.inventory(&self.hud_art, inv, gw, gh)
            }
            _ => {}
        }
        let gui_buffer = self.quads_buffer("gui", &gui_mesh.vertices);

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
            if let Some((indices, _)) = &self.indices {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                // Translucent chunks blend over what's behind them, so far ones go first.
                let mut keys: Vec<(i32, i32)> = self.chunks.keys().copied().collect();
                let distance = |&(cx, cz): &(i32, i32)| {
                    let center = Vec3::new(cx as f32 * 16.0 + 8.0, 0.0, cz as f32 * 16.0 + 8.0);
                    (center - camera.eye * Vec3::new(1.0, 0.0, 1.0)).length_squared()
                };
                keys.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
                // Portals are opaque, so they draw before the translucent layer.
                let order = [
                    (&self.opaque, 0),
                    (&self.cutout, 1),
                    (&self.portal, 3),
                    (&self.translucent, 2),
                ];
                pass.set_bind_group(1, &self.portal_bind_group, &[]);
                for (pipeline, slot) in order {
                    pass.set_pipeline(pipeline);
                    for key in &keys {
                        if let Some((buffer, count)) = &self.chunks[key][slot] {
                            pass.set_vertex_buffer(0, buffer.slice(..));
                            pass.draw_indexed(0..*count, 0, 0..1);
                        }
                    }
                    // Entities are cutout too, and go before anything translucent covers
                    // them.
                    if slot == 1
                        && let Some((buffer, count)) = &self.entity_vertices
                    {
                        pass.set_pipeline(&self.entity);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw_indexed(0..*count, 0, 0..1);
                    }
                    // `RenderPipelines.EYES`: blended over the bodies, fully bright, without
                    // writing depth.
                    if slot == 1
                        && let Some((buffer, count)) = &self.eye_vertices
                    {
                        pass.set_pipeline(&self.translucent);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw_indexed(0..*count, 0, 0..1);
                    }
                    // Name tags: their text where nothing hides it (`RenderPipelines.TEXT`),
                    // then their see-through parts over everything so far, the text too, as
                    // vanilla's see-through phase comes after the solid one.
                    if slot == 1 {
                        for (pipeline, vertices) in [
                            (&self.entity, &self.tag_vertices),
                            (&self.see_through, &self.see_through_vertices),
                        ] {
                            if let Some((buffer, count)) = vertices {
                                pass.set_pipeline(pipeline);
                                pass.set_vertex_buffer(0, buffer.slice(..));
                                pass.draw_indexed(0..*count, 0, 0..1);
                            }
                        }
                    }
                }
                if let Some((buffer, count)) = &self.crack_vertices {
                    pass.set_pipeline(&self.crack);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw_indexed(0..*count, 0, 0..1);
                }
                if let Some((buffer, count)) = &self.outline_vertices {
                    pass.set_pipeline(&self.outline);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw_indexed(0..*count, 0, 0..1);
                }
            }
            // Vanilla blurs the crosshair away under a screen; without the blur, it's hidden.
            if view.hud && self.screen.is_none() {
                pass.set_pipeline(&self.crosshair);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some((indices, _)) = &self.indices
            && (self.hand_vertices.is_some() || self.fire_overlay_vertices.is_some())
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().expect("made above").0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.hand_bind_group, &[]);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            // The hands, then the screen effects over them.
            for (pipeline, vertices) in [
                (&self.entity, &self.hand_vertices),
                (&self.see_through, &self.fire_overlay_vertices),
            ] {
                if let Some((buffer, count)) = vertices {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw_indexed(0..*count, 0, 0..1);
                }
            }
        }
        if gui_buffer.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.as_ref().expect("made above").0,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            if let (Some((buffer, count)), Some((indices, _))) = (&gui_buffer, &self.indices) {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_pipeline(&self.gui);
                pass.set_vertex_buffer(0, buffer.slice(..));
                // Unclipped runs between the clipped ones draw over the whole screen.
                let mut runs = Vec::new();
                let mut next = 0;
                for (range, rect) in &gui_mesh.clips {
                    runs.push((next..range.start, None));
                    runs.push((range.clone(), Some(rect)));
                    next = range.end;
                }
                runs.push((next..count / 6, None));
                for (quads, rect) in runs {
                    if quads.is_empty() {
                        continue;
                    }
                    let (x, y, w, h) = match rect {
                        Some(&[x, y, w, h]) => {
                            let s = scale as f32;
                            let x0 = ((x * s) as u32).min(width);
                            let y0 = ((y * s) as u32).min(height);
                            let x1 = (((x + w) * s) as u32).min(width);
                            let y1 = (((y + h) * s) as u32).min(height);
                            (x0, y0, x1 - x0, y1 - y0)
                        }
                        None => (0, 0, width, height),
                    };
                    pass.set_scissor_rect(x, y, w, h);
                    pass.draw_indexed(quads.start * 6..quads.end * 6, 0, 0..1);
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
