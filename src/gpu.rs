//! wgpu (DX12) renderer: HDR scene targets, crossfade, bloom chain, ACES tonemap, egui.

use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const BLOOM_LEVELS: usize = 6;
pub const MAX_PARTICLES: u32 = 1 << 17;
pub const SPEC_ROWS: u32 = 256;
pub const SPEC_COLS: u32 = 64;
pub const AUDIO_FLOATS: usize = 64 + 64 + 512;
pub const SCENE_NAMES: [&str; 11] = [
    "Halo",
    "Nebula",
    "Silk",
    "Plasma",
    "Molten Metal",
    "Ferrofluid",
    "Liquid Ether",
    "Light Tunnel",
    "Scanner",
    "Dither",
    "Faulty Terminal",
];
pub const SCENE_COUNT: usize = SCENE_NAMES.len();
pub const SCENE_NEBULA: usize = 1;
pub const SCENE_ETHER: usize = 6;
pub const SPLAT_VEC4S: usize = 16;

const COMMON: &str = include_str!("shaders/common.wgsl");
const NEBULA: &str = include_str!("shaders/nebula.wgsl");
const ETHER: &str = include_str!("shaders/ether.wgsl");
const POST: &str = include_str!("shaders/post.wgsl");

/// Single-pass fullscreen scenes: (scene index, label, source, fragment entry point).
const FULLSCREEN_SCENES: [(usize, &str, &str, &str); 9] = [
    (0, "halo", include_str!("shaders/halo.wgsl"), "fs_halo"),
    (2, "silk", include_str!("shaders/silk.wgsl"), "fs_silk"),
    (3, "plasma", include_str!("shaders/plasma.wgsl"), "fs_plasma"),
    (4, "molten", include_str!("shaders/molten.wgsl"), "fs_molten"),
    (5, "ferro", include_str!("shaders/ferro.wgsl"), "fs_ferro"),
    (7, "tunnel", include_str!("shaders/tunnel.wgsl"), "fs_tunnel"),
    (8, "scanner", include_str!("shaders/scanner.wgsl"), "fs_scanner"),
    (9, "dither", include_str!("shaders/dither.wgsl"), "fs_dither"),
    (10, "terminal", include_str!("shaders/terminal.wgsl"), "fs_terminal"),
];

/// Liquid Ether simulation grid height (width follows the window aspect).
const FLUID_H: u32 = 200;
const FLUID_JACOBI: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Globals {
    pub res: [f32; 2],
    pub inv_res: [f32; 2],
    pub time: f32,
    pub dt: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub energy: f32,
    pub beat: f32,
    pub beat_count: f32,
    pub flux: f32,
    pub intensity: f32,
    pub scene_time: f32,
    pub hat: f32,
    pub beat_strength: f32,
    pub overlay: f32,
    pub trails: f32,
    pub spec_pos: f32,
    pub bloom_threshold: f32,
    pub bloom_knee: f32,
    pub bloom_strength: f32,
    pub exposure: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub vignette: f32,
    pub transition: f32,
    pub bass_slow: f32,
    pub particle_count: f32,
    pub beat_trigger: f32,
    pub shape_morph: f32,
    pub cam_angle: f32,
    pub art_present: f32,
    pub kick: f32,
    pub travel: f32,
    pub overlay_opacity: f32,
    pub art_spin: f32,
    pub scroll: f32,
    pub flow: f32,
    pub beat_phase: f32,
    pub bpm: f32,
    pub _pad1: f32,
    pub _pad2: f32,
    pub pal: [[f32; 4]; 4],
    pub ring_age: [f32; 4],
    pub ring_str: [f32; 4],
}

pub struct Tex {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
}

fn make_tex(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> Tex {
    make_tex_with(device, w, h, format, wgpu::TextureUsages::empty(), label)
}

fn make_tex_with(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    extra: wgpu::TextureUsages,
    label: &str,
) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Tex { tex, view }
}

const VEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
const SCALAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Liquid Ether simulation state. Each frame: advect A->B, curl(B), force(B, curl)->A,
/// div(A), Jacobi pA<->pB, grad(pA, A)->B, then copy B->A so A always holds the result.
struct Fluid {
    vel: [Tex; 2],
    prs: [Tex; 2],
    div: Tex,
    curl: Tex,
    bg_advect: wgpu::BindGroup,
    bg_curl: wgpu::BindGroup,
    bg_force: wgpu::BindGroup,
    bg_div: wgpu::BindGroup,
    bg_jacobi: [wgpu::BindGroup; 2],
    bg_grad: wgpu::BindGroup,
    bg_display: wgpu::BindGroup,
    size: wgpu::Extent3d,
}

impl Fluid {
    fn new(device: &wgpu::Device, tex1: &wgpu::BindGroupLayout, tex2: &wgpu::BindGroupLayout, scene_w: u32, scene_h: u32) -> Self {
        let h = FLUID_H;
        let w = ((scene_w as f32 / scene_h.max(1) as f32) * h as f32).round().max(16.0) as u32;
        let copy = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
        let vel = [
            make_tex_with(device, w, h, VEL_FORMAT, copy, "vel-a"),
            make_tex_with(device, w, h, VEL_FORMAT, copy, "vel-b"),
        ];
        let prs = [make_tex(device, w, h, SCALAR_FORMAT, "prs-a"), make_tex(device, w, h, SCALAR_FORMAT, "prs-b")];
        let div = make_tex(device, w, h, SCALAR_FORMAT, "div");
        let curl = make_tex(device, w, h, SCALAR_FORMAT, "curl");
        let bg2 = |a: &wgpu::TextureView, b: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: tex2,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
                ],
            })
        };
        let bg_display = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: tex1,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&vel[0].view) }],
        });
        Self {
            bg_advect: bg2(&vel[0].view, &vel[0].view),
            bg_curl: bg2(&vel[1].view, &vel[1].view),
            bg_force: bg2(&vel[1].view, &curl.view),
            bg_div: bg2(&vel[0].view, &vel[0].view),
            bg_jacobi: [bg2(&prs[0].view, &div.view), bg2(&prs[1].view, &div.view)],
            bg_grad: bg2(&prs[0].view, &vel[0].view),
            bg_display,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            vel,
            prs,
            div,
            curl,
        }
    }
}

struct Targets {
    w: u32,
    h: u32,
    scene: [Tex; 2],
    hdr: Tex,
    trail: Tex,
    bloom: Vec<Tex>,
    bg_trail: wgpu::BindGroup,
    /// [A = slot0, B = slot1], [A = slot1, B = slot0]
    bg_comp: [wgpu::BindGroup; 2],
    bg_down: Vec<wgpu::BindGroup>,
    bg_up: Vec<wgpu::BindGroup>,
    bg_tonemap: wgpu::BindGroup,
    /// Without a crossfade, bloom and tonemap read the scene slot directly (no composite pass).
    bg_down_scene: [wgpu::BindGroup; 2],
    bg_tonemap_scene: [wgpu::BindGroup; 2],
    fluid: Fluid,
}

struct Pipelines {
    scenes: Vec<Option<wgpu::RenderPipeline>>,
    fl_advect: wgpu::RenderPipeline,
    fl_curl: wgpu::RenderPipeline,
    fl_force: wgpu::RenderPipeline,
    fl_div: wgpu::RenderPipeline,
    fl_jacobi: wgpu::RenderPipeline,
    fl_grad: wgpu::RenderPipeline,
    ether: wgpu::RenderPipeline,
    nebula_comp: wgpu::RenderPipeline,
    fade: wgpu::RenderPipeline,
    particle: wgpu::RenderPipeline,
    particle_cs: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    down_first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    tonemap: wgpu::RenderPipeline,
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: Option<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    /// Format render passes write to the swapchain with (sRGB view).
    pub view_format: wgpu::TextureFormat,
    pub transparent_ok: bool,
    pub adapter_name: String,
    pub render_scale: f32,

    globals_buf: wgpu::Buffer,
    audio_buf: wgpu::Buffer,
    splat_buf: wgpu::Buffer,
    spec_tex: wgpu::Texture,
    art_tex: wgpu::Texture,
    g0: wgpu::BindGroup,
    tex1_layout: wgpu::BindGroupLayout,
    tex2_layout: wgpu::BindGroupLayout,
    samp_clamp: wgpu::Sampler,
    particles: wgpu::Buffer,
    particle_bg: wgpu::BindGroup,
    pipes: Pipelines,
    targets: Targets,
    trail_dirty: bool,
    /// Area of the swapchain the scene is drawn into (x, y, w, h in physical pixels); the rest
    /// belongs to docked UI panels.
    viewport: [u32; 4],
    pub egui: egui_wgpu::Renderer,
}

pub struct FrameDesc {
    pub cur_scene: usize,
    pub cur_slot: usize,
    pub prev_scene: Option<usize>,
    pub particle_count: u32,
}

impl Gpu {
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<wgpu::Surface<'static>>,
        width: u32,
        height: u32,
        render_scale: f32,
    ) -> Result<Self, String> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: surface.as_ref(),
            })
            .await
            .map_err(|e| format!("No DirectX 12 adapter found: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("blazeaudio"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| format!("Failed to create device: {e}"))?;

        // ---- Surface ----
        let (config, view_format, transparent_ok) = match &surface {
            Some(s) => {
                let caps = s.get_capabilities(&adapter);
                let base = caps
                    .formats
                    .iter()
                    .copied()
                    .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
                    .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
                    .unwrap_or(caps.formats[0]);
                let srgb = base.add_srgb_suffix();
                let transparent_ok = caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied);
                let alpha_mode = if transparent_ok {
                    wgpu::CompositeAlphaMode::PreMultiplied
                } else {
                    caps.alpha_modes[0]
                };
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: base,
                    width: width.max(1),
                    height: height.max(1),
                    present_mode: wgpu::PresentMode::AutoVsync,
                    desired_maximum_frame_latency: 2,
                    alpha_mode,
                    view_formats: if srgb != base { vec![srgb] } else { vec![] },
                };
                s.configure(&device, &config);
                (config, srgb, transparent_ok)
            }
            None => {
                let fmt = wgpu::TextureFormat::Rgba8Unorm;
                (
                    wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: fmt,
                        width,
                        height,
                        present_mode: wgpu::PresentMode::AutoVsync,
                        desired_maximum_frame_latency: 2,
                        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                        view_formats: vec![],
                    },
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    false,
                )
            }
        };

        // ---- Shared resources ----
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let audio_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("audio"),
            size: (AUDIO_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let splat_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splats"),
            size: (SPLAT_VEC4S * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let spec_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spectrogram"),
            size: wgpu::Extent3d { width: SPEC_COLS, height: SPEC_ROWS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let art_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("album-art"),
            size: wgpu::Extent3d {
                width: crate::media::ART_SIZE,
                height: crate::media::ART_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let samp_wrap = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wrap"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let samp_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let vis = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
        let tex_entry = |binding: u32, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let g0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("g0"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: vis,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: vis,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(2, vis),
                samp_entry(3),
                tex_entry(4, vis),
                samp_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: vis,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let spec_view = spec_tex.create_view(&Default::default());
        let art_view = art_tex.create_view(&Default::default());
        let g0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("g0"),
            layout: &g0_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: audio_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&spec_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samp_wrap) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&art_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&samp_clamp) },
                wgpu::BindGroupEntry { binding: 6, resource: splat_buf.as_entire_binding() },
            ],
        });
        let tex1_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex1"),
            entries: &[tex_entry(0, wgpu::ShaderStages::FRAGMENT)],
        });
        let tex2_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex2"),
            entries: &[tex_entry(0, wgpu::ShaderStages::FRAGMENT), tex_entry(1, wgpu::ShaderStages::FRAGMENT)],
        });
        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // ---- Particles: seeded on a sphere ----
        let particles = {
            let mut data: Vec<f32> = Vec::with_capacity(MAX_PARTICLES as usize * 8);
            let mut rng: u32 = 0x9E3779B9;
            let mut rnd = || {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                rng as f32 / u32::MAX as f32
            };
            for i in 0..MAX_PARTICLES {
                let u = rnd() * std::f32::consts::TAU;
                let v = rnd() * 2.0 - 1.0;
                let s = (1.0 - v * v).sqrt();
                let r = 0.2 + rnd() * 0.3;
                data.extend_from_slice(&[u.cos() * s * r, v * r, u.sin() * s * r, 0.0, 0.0, 0.0, 0.0]);
                data.push(i as f32 / MAX_PARTICLES as f32 * 997.0 + rnd());
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particles"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            })
        };
        let particle_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particles"),
            layout: &particle_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: particles.as_entire_binding() }],
        });

        // ---- Pipelines (compiled in parallel; FXC is slow and single-threaded per shader) ----
        let pl_g0 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&g0_layout],
            push_constant_ranges: &[],
        });
        let pl_tex1 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&g0_layout, &tex1_layout],
            push_constant_ranges: &[],
        });
        let pl_tex2 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&g0_layout, &tex2_layout],
            push_constant_ranges: &[],
        });
        let pl_particles = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&g0_layout, &particle_layout],
            push_constant_ranges: &[],
        });

        let module = |name: &str, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{COMMON}\n{src}"))),
            })
        };
        let m_nebula = module("nebula", NEBULA);
        let m_ether = module("ether", ETHER);
        let m_post = module("post", POST);
        let m_scenes: Vec<(usize, wgpu::ShaderModule, &str)> =
            FULLSCREEN_SCENES.iter().map(|(i, name, src, fs)| (*i, module(name, src), *fs)).collect();

        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let fade_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let full = |layout: &wgpu::PipelineLayout,
                    m: &wgpu::ShaderModule,
                    fs: &str,
                    format: wgpu::TextureFormat,
                    blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: m,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: m,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview: None,
                cache: None,
            })
        };

        let pipes = std::thread::scope(|s| {
            // One thread per scene: FXC compiles each shader single-threaded.
            let h_scenes: Vec<_> = m_scenes
                .iter()
                .map(|(i, m, fs)| {
                    let (full, pl) = (&full, &pl_g0);
                    (*i, s.spawn(move || full(pl, m, fs, HDR_FORMAT, None)))
                })
                .collect();
            let h_fluid = s.spawn(|| {
                (
                    full(&pl_tex2, &m_ether, "fs_fl_advect", VEL_FORMAT, None),
                    full(&pl_tex2, &m_ether, "fs_fl_curl", SCALAR_FORMAT, None),
                    full(&pl_tex2, &m_ether, "fs_fl_force", VEL_FORMAT, None),
                    full(&pl_tex2, &m_ether, "fs_fl_div", SCALAR_FORMAT, None),
                    full(&pl_tex2, &m_ether, "fs_fl_jacobi", SCALAR_FORMAT, None),
                    full(&pl_tex2, &m_ether, "fs_fl_grad", VEL_FORMAT, None),
                    full(&pl_tex1, &m_ether, "fs_ether", HDR_FORMAT, None),
                )
            });
            let h_neb = s.spawn(|| {
                let nebula_comp = full(&pl_tex1, &m_nebula, "fs_nebula", HDR_FORMAT, None);
                let fade = full(&pl_g0, &m_nebula, "fs_fade", HDR_FORMAT, Some(fade_blend));
                let particle = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("particles"),
                    layout: Some(&pl_g0),
                    vertex: wgpu::VertexState {
                        module: &m_nebula,
                        entry_point: Some("vs_particle"),
                        compilation_options: Default::default(),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: 32,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                        }],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &m_nebula,
                        entry_point: Some("fs_particle"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: HDR_FORMAT,
                            blend: Some(additive),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                });
                let particle_cs = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("particle-cs"),
                    layout: Some(&pl_particles),
                    module: &m_nebula,
                    entry_point: Some("cs_update"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                (nebula_comp, fade, particle, particle_cs)
            });
            let h_post = s.spawn(|| {
                let composite = full(&pl_tex2, &m_post, "fs_composite", HDR_FORMAT, None);
                let down_first = full(&pl_tex1, &m_post, "fs_down_first", HDR_FORMAT, None);
                let down = full(&pl_tex1, &m_post, "fs_down", HDR_FORMAT, None);
                let up = full(&pl_tex1, &m_post, "fs_up", HDR_FORMAT, Some(additive));
                let tonemap = full(&pl_tex2, &m_post, "fs_tonemap", view_format, None);
                (composite, down_first, down, up, tonemap)
            });
            let (nebula_comp, fade, particle, particle_cs) = h_neb.join().unwrap();
            let (composite, down_first, down, up, tonemap) = h_post.join().unwrap();
            let (fl_advect, fl_curl, fl_force, fl_div, fl_jacobi, fl_grad, ether) = h_fluid.join().unwrap();
            let mut scenes: Vec<Option<wgpu::RenderPipeline>> = (0..SCENE_COUNT).map(|_| None).collect();
            for (i, h) in h_scenes {
                scenes[i] = Some(h.join().unwrap());
            }
            Pipelines {
                scenes,
                fl_advect,
                fl_curl,
                fl_force,
                fl_div,
                fl_jacobi,
                fl_grad,
                ether,
                nebula_comp,
                fade,
                particle,
                particle_cs,
                composite,
                down_first,
                down,
                up,
                tonemap,
            }
        });

        let egui = egui_wgpu::Renderer::new(&device, view_format, egui_wgpu::RendererOptions::default());

        let (tw, th) = scaled(width, height, render_scale);
        let targets = build_targets(&device, &tex1_layout, &tex2_layout, tw, th);

        let gpu = Self {
            device,
            queue,
            surface,
            config,
            view_format,
            transparent_ok,
            adapter_name,
            render_scale,
            globals_buf,
            audio_buf,
            splat_buf,
            spec_tex,
            art_tex,
            g0,
            tex1_layout,
            tex2_layout,
            samp_clamp,
            particles,
            particle_bg,
            pipes,
            targets,
            trail_dirty: true,
            viewport: [0, 0, width.max(1), height.max(1)],
            egui,
        };
        // Clear spectrogram + art to black.
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.spec_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &vec![0u8; (SPEC_COLS * SPEC_ROWS) as usize],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SPEC_COLS), rows_per_image: None },
            wgpu::Extent3d { width: SPEC_COLS, height: SPEC_ROWS, depth_or_array_layers: 1 },
        );
        let _ = &gpu.samp_clamp;
        Ok(gpu)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        if let Some(s) = &self.surface {
            s.configure(&self.device, &self.config);
        }
        self.viewport = [0, 0, width, height];
        self.rebuild_targets();
    }

    /// Restrict the scene to a sub-rectangle of the window (clamped to the surface). Scene
    /// targets are sized to it, so a docked panel doesn't squash or hide the visuals.
    pub fn set_viewport(&mut self, x: u32, y: u32, w: u32, h: u32) {
        let (sw, sh) = (self.config.width, self.config.height);
        let x = x.min(sw.saturating_sub(16));
        let y = y.min(sh.saturating_sub(16));
        let vp = [x, y, w.clamp(16, sw - x), h.clamp(16, sh - y)];
        if vp != self.viewport {
            self.viewport = vp;
            self.rebuild_targets();
        }
    }

    pub fn set_render_scale(&mut self, scale: f32) {
        if (scale - self.render_scale).abs() > 1e-3 {
            self.render_scale = scale;
            self.rebuild_targets();
        }
    }

    fn rebuild_targets(&mut self) {
        let (tw, th) = scaled(self.viewport[2], self.viewport[3], self.render_scale);
        if tw != self.targets.w || th != self.targets.h {
            self.targets = build_targets(&self.device, &self.tex1_layout, &self.tex2_layout, tw, th);
            self.trail_dirty = true;
        }
    }

    pub fn scene_size(&self) -> (u32, u32) {
        (self.targets.w, self.targets.h)
    }

    pub fn reset_trails(&mut self) {
        self.trail_dirty = true;
    }

    pub fn upload(&self, globals: &Globals, audio: &[f32; AUDIO_FLOATS]) {
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(globals));
        self.queue.write_buffer(&self.audio_buf, 0, bytemuck::cast_slice(audio));
    }

    pub fn upload_splats(&self, splats: &[[f32; 4]; SPLAT_VEC4S]) {
        self.queue.write_buffer(&self.splat_buf, 0, bytemuck::cast_slice(splats));
    }

    /// One Liquid Ether simulation step (see `Fluid` for the pass order).
    fn step_fluid(&self, enc: &mut wgpu::CommandEncoder) {
        let f = &self.targets.fluid;
        let p = &self.pipes;
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        Self::fullscreen_pass(enc, &f.vel[1].view, clear, &p.fl_advect, &self.g0, Some(&f.bg_advect));
        Self::fullscreen_pass(enc, &f.curl.view, clear, &p.fl_curl, &self.g0, Some(&f.bg_curl));
        Self::fullscreen_pass(enc, &f.vel[0].view, clear, &p.fl_force, &self.g0, Some(&f.bg_force));
        Self::fullscreen_pass(enc, &f.div.view, clear, &p.fl_div, &self.g0, Some(&f.bg_div));
        for i in 0..FLUID_JACOBI {
            let (dst, bg) = if i % 2 == 0 { (&f.prs[1], &f.bg_jacobi[0]) } else { (&f.prs[0], &f.bg_jacobi[1]) };
            Self::fullscreen_pass(enc, &dst.view, clear, &p.fl_jacobi, &self.g0, Some(bg));
        }
        Self::fullscreen_pass(enc, &f.vel[1].view, clear, &p.fl_grad, &self.g0, Some(&f.bg_grad));
        enc.copy_texture_to_texture(f.vel[1].tex.as_image_copy(), f.vel[0].tex.as_image_copy(), f.size);
    }

    pub fn write_spec_row(&self, row: u32, data: &[u8; SPEC_COLS as usize]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.spec_tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: row % SPEC_ROWS, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SPEC_COLS), rows_per_image: None },
            wgpu::Extent3d { width: SPEC_COLS, height: 1, depth_or_array_layers: 1 },
        );
    }

    pub fn upload_art(&self, rgba: &[u8]) {
        let n = crate::media::ART_SIZE;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.art_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 4), rows_per_image: None },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
    }

    fn fullscreen_pass(
        enc: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        pipeline: &wgpu::RenderPipeline,
        g0: &wgpu::BindGroup,
        g1: Option<&wgpu::BindGroup>,
    ) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, g0, &[]);
        if let Some(g1) = g1 {
            pass.set_bind_group(1, g1, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    fn draw_scene(&mut self, enc: &mut wgpu::CommandEncoder, scene: usize, slot: usize, particle_count: u32) {
        let t = &self.targets;
        if scene == SCENE_NEBULA {
            {
                let load = if self.trail_dirty {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                } else {
                    wgpu::LoadOp::Load
                };
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("trail"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.trail.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_bind_group(0, &self.g0, &[]);
                pass.set_pipeline(&self.pipes.fade);
                pass.draw(0..3, 0..1);
                pass.set_pipeline(&self.pipes.particle);
                pass.set_vertex_buffer(0, self.particles.slice(..));
                pass.draw(0..6, 0..particle_count.min(MAX_PARTICLES));
            }
            self.trail_dirty = false;
            Self::fullscreen_pass(
                enc,
                &t.scene[slot].view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.pipes.nebula_comp,
                &self.g0,
                Some(&t.bg_trail),
            );
        } else if scene == SCENE_ETHER {
            Self::fullscreen_pass(
                enc,
                &t.scene[slot].view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.pipes.ether,
                &self.g0,
                Some(&t.fluid.bg_display),
            );
        } else if let Some(p) = &self.pipes.scenes[scene] {
            Self::fullscreen_pass(enc, &t.scene[slot].view, wgpu::LoadOp::Clear(wgpu::Color::BLACK), p, &self.g0, None);
        }
    }

    /// Encode a full frame: scenes -> crossfade -> bloom -> tonemap into `out`.
    pub fn encode(&mut self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, fd: &FrameDesc, overlay: bool) {
        let needs_particles = fd.cur_scene == SCENE_NEBULA || fd.prev_scene == Some(SCENE_NEBULA);
        if needs_particles {
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particles"), timestamp_writes: None });
            cp.set_pipeline(&self.pipes.particle_cs);
            cp.set_bind_group(0, &self.g0, &[]);
            cp.set_bind_group(1, &self.particle_bg, &[]);
            cp.dispatch_workgroups(fd.particle_count.clamp(1, MAX_PARTICLES).div_ceil(256), 1, 1);
        }
        if fd.cur_scene == SCENE_ETHER || fd.prev_scene == Some(SCENE_ETHER) {
            self.step_fluid(enc);
        }
        if let Some(prev) = fd.prev_scene {
            self.draw_scene(enc, prev, fd.cur_slot ^ 1, fd.particle_count);
        }
        self.draw_scene(enc, fd.cur_scene, fd.cur_slot, fd.particle_count);

        // The composite pass only matters mid-crossfade; otherwise bloom and tonemap read the
        // current scene slot directly (they sanitize it themselves), saving a full-res pass.
        let t = &self.targets;
        let (bg_down0, bg_tonemap) = if fd.prev_scene.is_some() {
            Self::fullscreen_pass(
                enc,
                &t.hdr.view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                &self.pipes.composite,
                &self.g0,
                Some(&t.bg_comp[fd.cur_slot]),
            );
            (&t.bg_down[0], &t.bg_tonemap)
        } else {
            (&t.bg_down_scene[fd.cur_slot], &t.bg_tonemap_scene[fd.cur_slot])
        };

        // Bloom: progressive downsample, then tent upsample accumulating back up the chain.
        for i in 0..BLOOM_LEVELS {
            let (p, bg) = if i == 0 { (&self.pipes.down_first, bg_down0) } else { (&self.pipes.down, &t.bg_down[i]) };
            Self::fullscreen_pass(enc, &t.bloom[i].view, wgpu::LoadOp::Clear(wgpu::Color::BLACK), p, &self.g0, Some(bg));
        }
        for i in (1..BLOOM_LEVELS).rev() {
            Self::fullscreen_pass(enc, &t.bloom[i - 1].view, wgpu::LoadOp::Load, &self.pipes.up, &self.g0, Some(&t.bg_up[i]));
        }

        // Tonemap into the scene viewport. Outside it (behind docked panels) stays transparent
        // in overlay mode and black otherwise, so a translucent panel never shows the desktop.
        let clear = if overlay { wgpu::Color::TRANSPARENT } else { wgpu::Color::BLACK };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tonemap"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = self.viewport;
        // Full-surface viewports are left at the default, which also keeps the tiny offscreen
        // warm-up target in `step_offscreen` valid.
        if [x, y, w, h] != [0, 0, self.config.width, self.config.height] {
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        }
        pass.set_pipeline(&self.pipes.tonemap);
        pass.set_bind_group(0, &self.g0, &[]);
        pass.set_bind_group(1, bg_tonemap, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn encode_egui(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        out: &wgpu::TextureView,
        prims: &[egui::ClippedPrimitive],
        delta: &egui::TexturesDelta,
        screen: &egui_wgpu::ScreenDescriptor,
    ) {
        for (id, d) in &delta.set {
            self.egui.update_texture(&self.device, &self.queue, *id, d);
        }
        let cmds = self.egui.update_buffers(&self.device, &self.queue, enc, prims, screen);
        if !cmds.is_empty() {
            self.queue.submit(cmds);
        }
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            self.egui.render(&mut pass, prims, screen);
        }
        for id in &delta.free {
            self.egui.free_texture(id);
        }
    }

    /// Render one frame offscreen and return it as tightly packed RGBA8 (for --snapshot).
    pub fn capture(&mut self, fd: &FrameDesc) -> (u32, u32, Vec<u8>) {
        let (w, h) = (self.config.width, self.config.height);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.view_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let bpr = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode(&mut enc, &view, fd, false);
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let row = &data[(y * bpr) as usize..(y * bpr + w * 4) as usize];
            out.extend_from_slice(row);
        }
        drop(data);
        buf.unmap();
        (w, h, out)
    }

    /// Advance GPU state one frame without presenting (used by --snapshot warm-up).
    pub fn step_offscreen(&mut self, fd: &FrameDesc) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        let t = &self.targets;
        // Render into the hdr target as a throwaway output; it is overwritten by composite anyway.
        let scratch = t.scene[fd.cur_slot ^ 1].tex.create_view(&Default::default());
        let _ = scratch;
        let tmp = make_tex(&self.device, 4, 4, self.view_format, "warm");
        self.encode(&mut enc, &tmp.view, fd, false);
        self.queue.submit([enc.finish()]);
    }
}

fn scaled(w: u32, h: u32, s: f32) -> (u32, u32) {
    (((w as f32 * s) as u32).max(16), ((h as f32 * s) as u32).max(16))
}

fn build_targets(
    device: &wgpu::Device,
    tex1: &wgpu::BindGroupLayout,
    tex2: &wgpu::BindGroupLayout,
    w: u32,
    h: u32,
) -> Targets {
    let scene = [make_tex(device, w, h, HDR_FORMAT, "scene0"), make_tex(device, w, h, HDR_FORMAT, "scene1")];
    let hdr = make_tex(device, w, h, HDR_FORMAT, "hdr");
    let trail = make_tex(device, w, h, HDR_FORMAT, "trail");
    let bloom: Vec<Tex> = (0..BLOOM_LEVELS)
        .map(|i| make_tex(device, (w >> (i + 1)).max(1), (h >> (i + 1)).max(1), HDR_FORMAT, "bloom"))
        .collect();

    let bg1 = |v: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: tex1,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v) }],
        })
    };
    let bg2 = |a: &wgpu::TextureView, b: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: tex2,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
            ],
        })
    };
    let bg_trail = bg1(&trail.view);
    let bg_comp = [bg2(&scene[0].view, &scene[1].view), bg2(&scene[1].view, &scene[0].view)];
    let bg_down = (0..BLOOM_LEVELS)
        .map(|i| if i == 0 { bg1(&hdr.view) } else { bg1(&bloom[i - 1].view) })
        .collect();
    let bg_up = (0..BLOOM_LEVELS).map(|i| bg1(&bloom[i].view)).collect();
    let bg_tonemap = bg2(&hdr.view, &bloom[0].view);
    let bg_down_scene = [bg1(&scene[0].view), bg1(&scene[1].view)];
    let bg_tonemap_scene = [bg2(&scene[0].view, &bloom[0].view), bg2(&scene[1].view, &bloom[0].view)];
    let fluid = Fluid::new(device, tex1, tex2, w, h);
    Targets {
        w,
        h,
        scene,
        hdr,
        trail,
        bloom,
        bg_trail,
        bg_comp,
        bg_down,
        bg_up,
        bg_tonemap,
        bg_down_scene,
        bg_tonemap_scene,
        fluid,
    }
}
