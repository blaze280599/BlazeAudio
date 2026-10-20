//! Phosphor Scope: the beam is drawn into an afterglow buffer that fades a little each frame
//! (ping-ponged between two textures), then composited onto CRT glass.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use super::{Params, Scene, SceneConfig, SceneFrame, SceneSettings};
use crate::audio::WAVE_LEN;
use crate::gpu::{self, make_tex, GpuCtx, Tex, HDR_FORMAT};
use crate::ui::widgets::{choice, knob, num};
use crate::ui::Tokens;
use crate::vis::VisState;

/// Afterglow (beam intensity only; the palette is applied when compositing).
const PERSIST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// How the waveform is plotted (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Mode {
    /// Classic time sweep, triggered on a rising zero crossing. Also read for the X-Y mode
    /// earlier builds had, so their settings files still load.
    #[serde(alias = "Xy")]
    Yt,
    /// The sweep wrapped around a slowly turning circle.
    Polar,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// Afterglow time constant in seconds.
    pub persistence: f32,
    /// Beam width in pixels at 1440p.
    pub beam: f32,
    pub gain: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { mode: Mode::Yt, persistence: 0.08, beam: 3.0, gain: 1.0 }
    }
}

impl SceneConfig for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let d = Self::default();
        choice(ui, t, "Mode", &mut self.mode, d.mode, &[(Mode::Yt, "Y-T sweep"), (Mode::Polar, "Polar")]);
        knob(ui, t, "Persistence", &mut self.persistence, d.persistence, 0.02..=1.5, num(2, " s").log());
        knob(ui, t, "Beam width", &mut self.beam, d.beam, 1.0..=10.0, num(1, " px"));
        knob(ui, t, "Gain", &mut self.gain, d.gain, 0.25..=3.0, num(2, "×"));
    }

    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        self.mode = if r(0.0, 1.0) < 0.5 { Mode::Yt } else { Mode::Polar };
        // Log-uniform, so short and long afterglows are equally likely.
        self.persistence = 0.04 * 15f32.powf(r(0.0, 1.0));
        self.beam = r(1.5, 6.0);
        self.gain = r(0.7, 1.6);
    }
}

/// Must match `Params` in scope.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScopeParams {
    mode: f32,
    /// Afterglow kept per frame.
    decay: f32,
    beam: f32,
    gain: f32,
    /// First sample of the triggered Y-T sweep.
    trigger: f32,
    points: f32,
    _pad: [f32; 2],
}

/// The two afterglow textures and a bind group reading each.
struct Persist {
    tex: [Tex; 2],
    bg: [wgpu::BindGroup; 2],
}

impl Persist {
    fn new(gpu: &GpuCtx, (w, h): (u32, u32)) -> Self {
        let tex = [make_tex(gpu.device, w, h, PERSIST_FORMAT, "persist"), make_tex(gpu.device, w, h, PERSIST_FORMAT, "persist")];
        let bg = [gpu.tex1(&tex[0].view), gpu.tex1(&tex[1].view)];
        Self { tex, bg }
    }

    fn view(&self, i: usize) -> &wgpu::TextureView {
        &self.tex[i].view
    }
}

pub struct Scope {
    decay: wgpu::RenderPipeline,
    beam: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    persist: Persist,
    /// Which `persist` texture holds the newest afterglow.
    idx: usize,
}

impl Scope {
    pub fn new(gpu: &GpuCtx, size: (u32, u32)) -> Self {
        let m = gpu.module("scope", include_str!("../shaders/scope.wgsl"));
        let pl_tex1 = gpu.layout(true);
        // Beam: one instanced quad per segment, max-blended so overlapping segments don't
        // stack into a blown-out blob where the trace doubles back.
        let max = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        };
        let beam = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vs_beam"),
            layout: Some(&gpu.layout(false)),
            vertex: wgpu::VertexState { module: &m, entry_point: Some("vs_beam"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &m,
                entry_point: Some("fs_beam"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PERSIST_FORMAT,
                    blend: Some(wgpu::BlendState { color: max, alpha: max }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        Self {
            decay: gpu.fullscreen(&pl_tex1, &m, "fs_decay", PERSIST_FORMAT, None),
            beam,
            composite: gpu.fullscreen(&pl_tex1, &m, "fs_scope", HDR_FORMAT, None),
            persist: Persist::new(gpu, size),
            idx: 0,
        }
    }
}

impl Scene for Scope {
    fn resize(&mut self, gpu: &GpuCtx, size: (u32, u32)) {
        self.persist = Persist::new(gpu, size);
    }

    fn params(&mut self, vis: &VisState, settings: &SceneSettings, p: &mut Params) {
        let s = &settings.scope;
        p.set(&ScopeParams {
            mode: s.mode as u32 as f32,
            // Louder passages leave longer trails.
            decay: (-vis.dt / (s.persistence * (1.0 + vis.energy * 0.6)).max(1e-3)).exp(),
            beam: s.beam,
            gain: s.gain,
            trigger: trigger(&vis.wave) as f32,
            points: (WAVE_LEN / 2) as f32,
            _pad: [0.0; 2],
        });
    }

    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        self.idx ^= 1;
        let (cur, prev) = (self.idx, self.idx ^ 1);
        let black = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        // Afterglow: last frame faded (or nothing, right after switching here)...
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scope"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.persist.view(cur),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: black, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, f.g0, &[]);
        if !f.fresh {
            pass.set_pipeline(&self.decay);
            pass.set_bind_group(1, &self.persist.bg[prev], &[]);
            pass.draw(0..3, 0..1);
        }
        // ...then this frame's beam on top.
        pass.set_pipeline(&self.beam);
        pass.draw(0..6, 0..WAVE_LEN as u32 - 1);
        drop(pass);
        gpu::fullscreen_pass(enc, f.target, black, &self.composite, f.g0, Some(&self.persist.bg[cur]));
    }
}

/// Start of a stable Y-T sweep: the first rising zero crossing in the first half of the window
/// that comes out of a clearly negative stretch (so noise around zero doesn't retrigger).
fn trigger(wave: &[f32; WAVE_LEN]) -> usize {
    let mut armed = false;
    for i in 1..WAVE_LEN / 2 {
        if wave[i] < -0.05 {
            armed = true;
        } else if armed && wave[i - 1] < 0.0 && wave[i] >= 0.0 {
            return i;
        }
    }
    0
}
