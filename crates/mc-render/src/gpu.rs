//! Draws a meshed world with wgpu, into a window's surface or an offscreen image.

use crate::bake::BlockModels;
use crate::mesh::{Mesh, Vertex};
use anyhow::{Context, Result};
use glam::{EulerRot, Mat4, Quat, Vec3};
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

    /// `Camera.setRotation`: the camera's turn from looking down -z with y up, so it looks
    /// along `forward`. Built from the angles, it stays upright looking straight up or down.
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(
            EulerRot::YXZ,
            std::f32::consts::PI - self.yaw.to_radians(),
            -self.pitch.to_radians(),
            0.0,
        )
    }

    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        // wgpu's clip space is y-up with depth 0..1, which glam calls the DirectX convention.
        glam::camera::rh::proj::directx::perspective(self.fov.to_radians(), aspect, 0.05, far)
            * self.view()
    }

    /// World space to view space, bob and all.
    pub fn view(&self) -> Mat4 {
        self.bob * Mat4::from_quat(self.rotation().inverse()) * Mat4::from_translation(-self.eye)
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

/// What a camera inside water or lava sees (`FogRenderer.setupFog`): the fluid's fog,
/// which also narrows the field of view to 6/7 of it (`Camera.modifyFovBasedOnDeathOrFluid`),
/// and in water the underwater overlay (`ScreenEffectRenderer.submitWater`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InFluid {
    pub fog_color: [f32; 3],
    /// Where the fog starts and is complete, in blocks from the eye.
    pub fog_start: f32,
    pub fog_end: f32,
    /// The overlay texture's u and v offsets, and its brightness.
    pub overlay: Option<[f32; 3]>,
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
    /// The environment's fog start and end.
    fog_env: [f32; 4],
    /// The underwater overlay's offsets and brightness, and tan(half the hand's fov).
    water: [f32; 4],
    /// The lightmap's inputs (`LightmapRenderState`): the sky light's colour and factor,
    /// the ambient colour and the block light factor, and the block light's tint.
    light_sky: [f32; 4],
    light_ambient: [f32; 4],
    light_tint: [f32; 4],
    /// The clouds' colour, and where the cell under the eye starts, eye-relative, with
    /// the clouds' fog end (`CloudInfo` and `FogCloudsEnd`).
    cloud_color: [f32; 4],
    cloud_offset: [f32; 4],
    /// 1 to draw the dark disc, 1 when the sky is the overworld's.
    sky_flags: [f32; 4],
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// `BlendFunction.OVERLAY`: the source, by its alpha, added to what's there.
const OVERLAY: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

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
    dark_disc: wgpu::RenderPipeline,
    sunrise: wgpu::RenderPipeline,
    celestial: wgpu::RenderPipeline,
    stars_pipeline: wgpu::RenderPipeline,
    clouds_pipeline: wgpu::RenderPipeline,
    /// What the sky looks like, and the clouds.
    sky_state: crate::sky::SkyState,
    stars: Vec<[Vec3; 4]>,
    /// The sun's and moons' rects in the atlas, by sprite id.
    celestial_sprites: std::collections::HashMap<String, [f32; 4]>,
    clouds: crate::sky::Clouds,
    /// The cloud faces drawn, with the cell and side of the layer they were built for.
    cloud_faces: Option<(crate::sky::CloudKey, Option<(wgpu::Buffer, u32)>)>,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    entity: wgpu::RenderPipeline,
    translucent: wgpu::RenderPipeline,
    see_through: wgpu::RenderPipeline,
    portal: wgpu::RenderPipeline,
    outline: wgpu::RenderPipeline,
    crosshair: wgpu::RenderPipeline,
    gui_inverted: wgpu::RenderPipeline,
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
    in_fluid: Option<InFluid>,
    underwater: wgpu::RenderPipeline,
    underwater_bind_group: wgpu::BindGroup,
    /// `RenderPipelines.WEATHER`, and the rain's and snow's textures, each bound in the
    /// portal's sky slot.
    weather: wgpu::RenderPipeline,
    weather_bind_groups: [wgpu::BindGroup; 2],
    weather_vertices: [Option<(wgpu::Buffer, u32)>; 2],
    /// `RenderPipelines.VIGNETTE`, darkening the view by `misc/vignette.png`.
    vignette: wgpu::RenderPipeline,
    vignette_bind_group: wgpu::BindGroup,
    lightning: wgpu::RenderPipeline,
    lightning_vertices: Option<(wgpu::Buffer, u32)>,
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
        let underwater_texture = portal_texture("underwater", &portal_textures.underwater);
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
        let underwater_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("underwater"),
            layout: &portal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&underwater_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&underwater_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&repeat),
                },
            ],
        });
        let weather_bind_groups = [&portal_textures.rain, &portal_textures.snow].map(|img| {
            let view = portal_texture("weather", img);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("weather"),
                layout: &portal_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&repeat),
                    },
                ],
            })
        });
        // `blur` in its `.mcmeta`: filtered, stretched over the screen.
        let vignette_view = portal_texture("vignette", &portal_textures.vignette);
        let blur = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let vignette_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vignette"),
            layout: &portal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&vignette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vignette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&blur),
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
            let textured = portal || fragment == "fs_weather";
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(if textured {
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
        // The sky's pieces after its disc: the sunrise fan (`RenderPipelines.SUNRISE_SUNSET`,
        // blended), the sun and moon (`CELESTIAL`) and stars (`STARS`), added; then the dark
        // disc (`SKY`, under the horizon), all behind whatever the world draws.
        let sky_layer = |fragment: &str, blend: wgpu::BlendState, buffers: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if buffers { "vs" } else { "vs_sky" }),
                    buffers: if buffers {
                        std::slice::from_ref(&vertex_layout)
                    } else {
                        &[]
                    },
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
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sunrise = sky_layer("fs_sunrise", wgpu::BlendState::ALPHA_BLENDING, true);
        let celestial = sky_layer("fs_celestial", OVERLAY, true);
        let stars_pipeline = sky_layer("fs_stars", OVERLAY, true);
        let dark_disc = sky_layer("fs_dark_disc", wgpu::BlendState::REPLACE, false);
        // `RenderPipelines.CLOUDS`: each face a quad of six vertices from one instance's
        // packed face, blended, writing depth, culled.
        let cloud_layout = wgpu::VertexBufferLayout {
            array_stride: 4,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Uint32],
        };
        let clouds_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clouds"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cloud"),
                buffers: &[Some(cloud_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_cloud"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
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
        // `BlendFunction.LIGHTNING`: added to what's behind, as much as its alpha.
        let lightning = pipeline(
            "fs_lightning",
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            true,
            false,
            true,
        );
        // `WEATHER_SNIPPET`: translucent, both sides, tested against depth without writing it.
        let weather = pipeline(
            "fs_weather",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
            false,
            false,
        );
        let line_layout = Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32],
        });
        // Drawn over the world, tested against its depth but not writing it.
        let overlay = |layout: &wgpu::PipelineLayout,
                       vertex: &str,
                       fragment: &str,
                       buffers: &[Option<wgpu::VertexBufferLayout>],
                       blend: wgpu::BlendState,
                       depth_compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(layout),
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
            &pipeline_layout,
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
            &pipeline_layout,
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
            &pipeline_layout,
            "vs",
            "fs_translucent",
            vertex_layouts,
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::CompareFunction::Always,
        );
        // `RenderPipelines.BLOCK_SCREEN_EFFECT`: blended over the whole view.
        let underwater = overlay(
            &portal_pipeline_layout,
            "vs_sky",
            "fs_underwater",
            &[],
            wgpu::BlendState::ALPHA_BLENDING,
            wgpu::CompareFunction::Always,
        );
        // Its blend: what's behind times one minus the vignette's colour.
        let vignette = overlay(
            &portal_pipeline_layout,
            "vs_sky",
            "fs_vignette",
            &[],
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            wgpu::CompareFunction::Always,
        );
        // The HUD has a depth buffer of its own, so items' faces hide each other.
        let gui_pipeline = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                        blend: Some(blend),
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
            })
        };
        let gui = gui_pipeline("gui", wgpu::BlendState::ALPHA_BLENDING);
        // `RenderPipelines.CROSSHAIR`'s blend, for the attack indicator.
        let gui_inverted = gui_pipeline(
            "gui inverted",
            wgpu::BlendState {
                color: invert,
                alpha: wgpu::BlendComponent::OVER,
            },
        );
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
            dark_disc,
            sunrise,
            celestial,
            stars_pipeline,
            clouds_pipeline,
            sky_state: Default::default(),
            stars: crate::sky::stars(),
            celestial_sprites: crate::sky::sprites()
                .map(|id| {
                    let rect = atlas.sprite(&id).rect;
                    (id, rect)
                })
                .collect(),
            clouds: crate::sky::Clouds::new(&models.portal_textures.clouds),
            cloud_faces: None,
            opaque,
            cutout,
            entity,
            translucent,
            see_through,
            portal,
            outline,
            crosshair,
            gui_inverted,
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
            in_fluid: None,
            underwater,
            underwater_bind_group,
            weather,
            weather_bind_groups,
            weather_vertices: [None, None],
            vignette,
            vignette_bind_group,
            lightning,
            lightning_vertices: None,
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

    /// The time of day and what the sky and clouds look like.
    pub fn set_sky(&mut self, state: crate::sky::SkyState) {
        self.sky_state = state;
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
    /// Draws these rain and snow columns (from `weather::columns`) after the clouds.
    pub fn set_weather(&mut self, columns: &crate::weather::Columns) {
        self.weather_vertices = [
            self.quads_buffer("rain", &columns.rain),
            self.quads_buffer("snow", &columns.snow),
        ];
    }

    pub fn set_cracks(&mut self, vertices: &[Vertex]) {
        self.crack_vertices = self.quads_buffer("cracks", vertices);
    }

    /// Draws these entity quads (from `entity::vertices`) after the cutout blocks.
    pub fn set_entities(&mut self, meshes: &crate::entity::Meshes) {
        self.entity_vertices = self.quads_buffer("entities", &meshes.cutout);
        self.eye_vertices = self.quads_buffer("entity eyes", &meshes.eyes);
        self.see_through_vertices = self.quads_buffer("name tags behind", &meshes.see_through);
        self.tag_vertices = self.quads_buffer("name tags", &meshes.tags);
        self.lightning_vertices = self.quads_buffer("lightning", &meshes.lightning);
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
    pub fn set_in_fluid(&mut self, in_fluid: Option<InFluid>) {
        self.in_fluid = in_fluid;
    }

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
        let fluid = self.in_fluid;
        let fov_scale = if fluid.is_some() { 0.85714287 } else { 1.0 };
        let camera = &Camera {
            fov: camera.fov * fov_scale,
            ..*camera
        };
        let hand_fov = 70.0 * fov_scale;
        let state = self.sky_state;
        let v = &state.visuals;
        // `GameRenderer`'s depth far: four times the render distance, or the cloud range
        // (vanilla's default, 128 chunks).
        let far = (view.distance * 4.0).max(2048.0);
        let view_proj = camera.view_proj(width as f32 / height as f32, far);
        // Vanilla's terrain fog: it ends at the render distance and starts a tenth before.
        let fog_end = view.distance;
        let fog_start = fog_end - (fog_end / 10.0).clamp(4.0, 64.0);
        let chunks = view.distance / 16.0;
        // `AtmosphericFogEnvironment.getBaseColor`: looking toward the sunrise or sunset
        // takes on its colour, and the nearer the sky's fog ends, the more the fog takes on
        // the sky's colour, so short distances don't look washed out.
        let lerp3 = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        };
        let mut fog = v.fog_color;
        if chunks >= 4.0 {
            let sun_x = if v.sun_angle.to_radians().sin() > 0.0 {
                -1.0
            } else {
                1.0
            };
            let toward_sun = camera.forward().x * sun_x;
            let sunrise = v.sunrise_color;
            if toward_sun > 0.0 && sunrise[3] > 0.0 {
                let rgb = [sunrise[0], sunrise[1], sunrise[2]];
                fog = lerp3(fog, rgb, toward_sun * sunrise[3]);
            }
        }
        // `applyWeatherDarken`: the sky's colour in the fog darkens, blue least, in rain
        // and again in thunder.
        let rain_dark = [
            1.0 - state.rain * 0.5,
            1.0 - state.rain * 0.5,
            1.0 - state.rain * 0.4,
        ];
        let fog_sky: [f32; 3] =
            std::array::from_fn(|i| v.sky_color[i] * rain_dark[i] * (1.0 - state.thunder * 0.5));
        let sky_fog_chunks = (v.sky_fog_end / 16.0).min(chunks);
        let toward_sky = 1.0
            - (0.25 + 0.75 * sky_fog_chunks / 32.0)
                .clamp(0.25, 1.0)
                .powf(0.25);
        let fog = lerp3(fog, fog_sky, toward_sky);
        // `FogRenderer.computeFogColor`: below the world the fog fades to black.
        let fog = fog.map(|c| c * (1.0 - state.void_darkness).powi(2));
        let mut fog_color = [fog[0], fog[1], fog[2], 1.0];
        let sky = v.sky_color;
        // `AtmosphericFogEnvironment.setupFog`.
        let (mut fog_env, mut sky_end, mut water) = (
            // Rain pulls the fog in, though not nearer than 96 blocks.
            [
                v.fog_start - 160.0 * state.rain_fog,
                v.fog_end.min(96.0).max(v.fog_end - 256.0 * state.rain_fog),
                0.0,
                0.0,
            ],
            view.distance.min(v.sky_fog_end),
            [0.0; 4],
        );
        if let Some(f) = fluid {
            fog_color = [f.fog_color[0], f.fog_color[1], f.fog_color[2], 1.0];
            fog_env = [f.fog_start, f.fog_end, 0.0, 0.0];
            sky_end = f.fog_end;
            if let Some([u, v, brightness]) = f.overlay {
                water = [u, v, brightness, (hand_fov / 2.0f32).to_radians().tan()];
            }
        }
        // `CloudRenderer.prepare`: the faces around the cell under the eye, rebuilt when
        // the eye moves to another cell or to another side of the layer.
        let draw_clouds =
            v.cloud_color[3] > 0.0 && state.skybox == crate::environment::Skybox::Overworld;
        let eye64 = camera.eye.as_dvec3().to_array();
        let (cloud_key, cloud_offset) =
            self.clouds
                .place(eye64, v.cloud_height, state.game_time, state.partial_tick);
        if draw_clouds
            && self
                .cloud_faces
                .as_ref()
                .is_none_or(|(key, _)| *key != cloud_key)
        {
            let faces = self.clouds.faces(cloud_key, crate::sky::CLOUD_RADIUS);
            let buffer = (!faces.is_empty()).then(|| {
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("cloud faces"),
                        contents: bytemuck::cast_slice(&faces),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                (buffer, faces.len() as u32)
            });
            self.cloud_faces = Some((cloud_key, buffer));
        }
        // The sky's sunrise fan, sun, moon and stars, placed around the eye.
        let sky_mesh = {
            let sprites = &self.celestial_sprites;
            let sprite = |id: &str| sprites.get(id).copied().unwrap_or([0.0; 4]);
            crate::sky::mesh(&state, &sprite, &self.stars)
        };
        let sky_lists = [&sky_mesh.sunrise, &sky_mesh.celestial, &sky_mesh.stars];
        let sky_vertices: Vec<Vertex> = sky_lists
            .iter()
            .flat_map(|list| list.iter())
            .map(|vertex| Vertex {
                pos: (Vec3::from(vertex.pos) + camera.eye).to_array(),
                ..*vertex
            })
            .collect();
        let sky_buffer = (!sky_vertices.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sky"),
                    contents: bytemuck::cast_slice(&sky_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            eye: camera.eye.extend(1.0).into(),
            fog_color,
            // The sky fogs out at the render distance, or the fluid's fog (`FogData.skyEnd`).
            sky_color: [sky[0], sky[1], sky[2], sky_end],
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
            fog_env,
            water,
            light_sky: [
                v.sky_light_color[0],
                v.sky_light_color[1],
                v.sky_light_color[2],
                v.sky_light_factor,
            ],
            light_ambient: [
                v.ambient_color[0],
                v.ambient_color[1],
                v.ambient_color[2],
                state.block_factor,
            ],
            light_tint: [
                v.block_light_tint[0],
                v.block_light_tint[1],
                v.block_light_tint[2],
                state.vignette,
            ],
            cloud_color: v.cloud_color,
            cloud_offset: [
                cloud_offset.x,
                cloud_offset.y,
                cloud_offset.z,
                v.cloud_fog_end.min(2048.0),
            ],
            sky_flags: [
                f32::from(u8::from(state.dark_disc)),
                f32::from(u8::from(
                    state.skybox == crate::environment::Skybox::Overworld,
                )),
                0.0,
                0.0,
            ],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let hand_view_proj = Camera {
            fov: hand_fov,
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
        // The attack indicator goes with the crosshair, which a screen hides.
        let inverted_buffer = if view.hud && self.screen.is_none() {
            self.quads_buffer("gui inverted", &gui_mesh.inverted)
        } else {
            None
        };

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Vanilla clears to the fog's colour.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: fog_color[0] as f64,
                            g: fog_color[1] as f64,
                            b: fog_color[2] as f64,
                            a: 1.0,
                        }),
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
            if let Some(buffer) = &sky_buffer {
                pass.set_vertex_buffer(0, buffer.slice(..));
                let mut start = 0;
                for (pipeline, list) in [&self.sunrise, &self.celestial, &self.stars_pipeline]
                    .into_iter()
                    .zip(sky_lists)
                {
                    let end = start + list.len() as u32;
                    if end > start {
                        pass.set_pipeline(pipeline);
                        pass.draw(start..end, 0..1);
                    }
                    start = end;
                }
            }
            if state.dark_disc {
                pass.set_pipeline(&self.dark_disc);
                pass.draw(0..3, 0..1);
            }
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
                if let Some((buffer, count)) = &self.lightning_vertices {
                    pass.set_pipeline(&self.lightning);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw_indexed(0..*count, 0, 0..1);
                }
                if draw_clouds && let Some((_, Some((buffer, count)))) = &self.cloud_faces {
                    pass.set_pipeline(&self.clouds_pipeline);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..6, 0..*count);
                }
                // `WeatherEffectRenderer.render`: rain, then snow.
                for (vertices, group) in self.weather_vertices.iter().zip(&self.weather_bind_groups)
                {
                    if let Some((buffer, count)) = vertices {
                        pass.set_pipeline(&self.weather);
                        pass.set_bind_group(1, group, &[]);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw_indexed(0..*count, 0, 0..1);
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
            && (self.hand_vertices.is_some()
                || self.fire_overlay_vertices.is_some()
                || fluid.is_some_and(|f| f.overlay.is_some()))
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
            // The hands, then the screen effects over them: water, then fire.
            let draw =
                |pass: &mut wgpu::RenderPass, pipeline, vertices: &Option<(wgpu::Buffer, u32)>| {
                    if let Some((buffer, count)) = vertices {
                        pass.set_pipeline(pipeline);
                        pass.set_vertex_buffer(0, buffer.slice(..));
                        pass.draw_indexed(0..*count, 0, 0..1);
                    }
                };
            draw(&mut pass, &self.entity, &self.hand_vertices);
            if fluid.is_some_and(|f| f.overlay.is_some()) {
                pass.set_pipeline(&self.underwater);
                pass.set_bind_group(1, &self.underwater_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            draw(&mut pass, &self.see_through, &self.fire_overlay_vertices);
        }
        // `Hud.extractCameraOverlays`: the vignette first, under the rest of the HUD.
        let vignette = view.hud && state.vignette > 0.0;
        if gui_buffer.is_some() || inverted_buffer.is_some() || vignette {
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
            if vignette {
                pass.set_pipeline(&self.vignette);
                pass.set_bind_group(1, &self.vignette_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            if let (Some((buffer, count)), Some((indices, _))) = (&inverted_buffer, &self.indices) {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_pipeline(&self.gui_inverted);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw_indexed(0..*count, 0, 0..1);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(yaw: f32, pitch: f32) -> Camera {
        Camera {
            eye: Vec3::new(3.0, 7.0, -5.0),
            yaw,
            pitch,
            fov: 70.0,
            bob: Mat4::IDENTITY,
        }
    }

    /// The view looks along `forward` with the world upright, and looking straight up or
    /// down is the limit of looking nearly so, not a turned or broken view.
    #[test]
    fn the_view_looks_where_the_camera_faces_even_straight_down() {
        for (yaw, pitch) in [(0.0, 0.0), (37.0, -20.0), (-120.0, 45.0), (200.0, 90.0)] {
            let c = camera(yaw, pitch);
            let ahead = c.view().transform_point3(c.eye + c.forward());
            assert!(ahead.abs_diff_eq(-Vec3::Z, 1e-5), "{yaw} {pitch}: {ahead}");
        }
        for (yaw, pitch, near) in [(30.0, 90.0, 89.999), (-75.0, -90.0, -89.999)] {
            let (at, close) = (camera(yaw, pitch).view(), camera(yaw, near).view());
            assert!(at.abs_diff_eq(close, 1e-3), "{yaw} {pitch}:\n{at}\n{close}");
        }
        // Level, the world's up is the screen's up.
        let up = camera(37.0, 0.0).view().transform_vector3(Vec3::Y);
        assert!(up.abs_diff_eq(Vec3::Y, 1e-5), "{up}");
    }
}
