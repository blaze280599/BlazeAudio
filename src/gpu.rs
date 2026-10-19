//! wgpu (DX12) renderer: the Faulty Terminal scene into an HDR target, bloom chain, ACES
//! tonemap, egui.

use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const BLOOM_LEVELS: usize = 6;
pub const AUDIO_FLOATS: usize = 64;

const COMMON: &str = include_str!("shaders/common.wgsl");
const TERMINAL: &str = include_str!("shaders/terminal.wgsl");
const POST: &str = include_str!("shaders/post.wgsl");

/// Must match `Globals` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Globals {
    pub res: [f32; 2],
    pub inv_res: [f32; 2],
    pub time: f32,
    pub treble: f32,
    pub hat: f32,
    pub kick: f32,
    pub beat_count: f32,
    pub flow: f32,
    pub intensity: f32,
    pub overlay: f32,
    pub overlay_opacity: f32,
    pub bloom_threshold: f32,
    pub bloom_knee: f32,
    pub bloom_strength: f32,
    pub exposure: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub vignette: f32,
    pub pal: [[f32; 4]; 4],
}

pub struct Tex {
    /// Owns the texture behind `view`.
    _tex: wgpu::Texture,
    pub view: wgpu::TextureView,
}

fn make_tex(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Tex { _tex: tex, view }
}

struct Targets {
    w: u32,
    h: u32,
    scene: Tex,
    bloom: Vec<Tex>,
    /// Input of each bloom downsample level (level 0 reads the scene).
    bg_down: Vec<wgpu::BindGroup>,
    bg_up: Vec<wgpu::BindGroup>,
    /// Scene + bloom for the tonemap pass.
    bg_tonemap: wgpu::BindGroup,
}

struct Pipelines {
    scene: wgpu::RenderPipeline,
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
    g0: wgpu::BindGroup,
    tex1_layout: wgpu::BindGroupLayout,
    tex2_layout: wgpu::BindGroupLayout,
    pipes: Pipelines,
    targets: Targets,
    /// Area of the swapchain the scene is drawn into (x, y, w, h in physical pixels); the rest
    /// belongs to docked UI panels.
    viewport: [u32; 4],
    pub egui: egui_wgpu::Renderer,
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
        let samp_clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let vis = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: vis,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let g0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("g0"),
            layout: &g0_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: audio_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&samp_clamp) },
            ],
        });
        let tex1_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex1"),
            entries: &[tex_entry(0)],
        });
        let tex2_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex2"),
            entries: &[tex_entry(0), tex_entry(1)],
        });

        // ---- Pipelines (compiled in parallel; FXC is slow and single-threaded per shader) ----
        let pipeline_layout = |layouts: &[&wgpu::BindGroupLayout]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            })
        };
        let pl_g0 = pipeline_layout(&[&g0_layout]);
        let pl_tex1 = pipeline_layout(&[&g0_layout, &tex1_layout]);
        let pl_tex2 = pipeline_layout(&[&g0_layout, &tex2_layout]);

        let module = |name: &str, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{COMMON}\n{src}"))),
            })
        };
        let m_scene = module("terminal", TERMINAL);
        let m_post = module("post", POST);

        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
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
            let h_scene = s.spawn(|| full(&pl_g0, &m_scene, "fs_terminal", HDR_FORMAT, None));
            let h_post = s.spawn(|| {
                let down_first = full(&pl_tex1, &m_post, "fs_down_first", HDR_FORMAT, None);
                let down = full(&pl_tex1, &m_post, "fs_down", HDR_FORMAT, None);
                let up = full(&pl_tex1, &m_post, "fs_up", HDR_FORMAT, Some(additive));
                let tonemap = full(&pl_tex2, &m_post, "fs_tonemap", view_format, None);
                (down_first, down, up, tonemap)
            });
            let (down_first, down, up, tonemap) = h_post.join().unwrap();
            Pipelines { scene: h_scene.join().unwrap(), down_first, down, up, tonemap }
        });

        let egui = egui_wgpu::Renderer::new(&device, view_format, egui_wgpu::RendererOptions::default());

        let (tw, th) = scaled(width, height, render_scale);
        let targets = build_targets(&device, &tex1_layout, &tex2_layout, tw, th);

        Ok(Self {
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
            g0,
            tex1_layout,
            tex2_layout,
            pipes,
            targets,
            viewport: [0, 0, width.max(1), height.max(1)],
            egui,
        })
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
        }
    }

    pub fn scene_size(&self) -> (u32, u32) {
        (self.targets.w, self.targets.h)
    }

    pub fn upload(&self, globals: &Globals, audio: &[f32; AUDIO_FLOATS]) {
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(globals));
        self.queue.write_buffer(&self.audio_buf, 0, bytemuck::cast_slice(audio));
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

    /// Encode a full frame: scene -> bloom -> tonemap into `out`.
    pub fn encode(&mut self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, overlay: bool) {
        let t = &self.targets;
        let black = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        Self::fullscreen_pass(enc, &t.scene.view, black, &self.pipes.scene, &self.g0, None);

        // Bloom: progressive downsample, then tent upsample accumulating back up the chain.
        for i in 0..BLOOM_LEVELS {
            let p = if i == 0 { &self.pipes.down_first } else { &self.pipes.down };
            Self::fullscreen_pass(enc, &t.bloom[i].view, black, p, &self.g0, Some(&t.bg_down[i]));
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
        pass.set_bind_group(1, &t.bg_tonemap, &[]);
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
    pub fn capture(&mut self) -> (u32, u32, Vec<u8>) {
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
        self.encode(&mut enc, &view, false);
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

    /// Render a frame without presenting (used by --snapshot to time the pipeline).
    pub fn step_offscreen(&mut self) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        let tmp = make_tex(&self.device, 4, 4, self.view_format, "warm");
        self.encode(&mut enc, &tmp.view, false);
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
    let scene = make_tex(device, w, h, HDR_FORMAT, "scene");
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
    let bg_down = (0..BLOOM_LEVELS)
        .map(|i| if i == 0 { bg1(&scene.view) } else { bg1(&bloom[i - 1].view) })
        .collect();
    let bg_up = (0..BLOOM_LEVELS).map(|i| bg1(&bloom[i].view)).collect();
    let bg_tonemap = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: tex2,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&bloom[0].view) },
        ],
    });
    Targets { w, h, scene, bloom, bg_down, bg_up, bg_tonemap }
}
