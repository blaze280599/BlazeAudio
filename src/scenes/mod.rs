//! The visuals. Each scene is one module here: its pipelines and passes (`Scene`), the values
//! it uploads each frame (`Params`), and its own settings with their drawer UI (`SceneConfig`).
//! Everything shared (globals, audio, bloom, tonemap, post effects) stays in `gpu`.
//!
//! Adding a scene:
//! 1. Write `src/shaders/<name>.wgsl`. common.wgsl is prepended, so `G` (globals), `A` (audio),
//!    `samp_clamp`, `vs_full` and the helpers are there. If the scene has values of its own,
//!    declare them as `struct Params { .. }` and `@group(0) @binding(3) var<uniform> P: Params;`.
//! 2. Add `src/scenes/<name>.rs` with a type implementing `Scene` (and, for settings, a
//!    `Settings` struct implementing `SceneConfig`). terminal.rs is the minimal example.
//! 3. Here: a `mod` line, a `SceneId` variant (also in `ALL`), its arm in `SceneId::build` and
//!    `SceneId::name`, and, if it has settings, a field in `SceneSettings` and its arm in
//!    `SceneSettings::config`.
//!
//! The sidebar buttons, number keys, Next scene, auto-switch, Randomize and Reset all pick the
//! new scene up from there.

pub mod ascii;
pub mod scope;
mod terminal;

use bytemuck::Pod;
use serde::{Deserialize, Serialize};

use crate::gpu::GpuCtx;
use crate::ui::Tokens;
use crate::vis::VisState;

/// Which visual is on screen. Variant names are what settings.json stores.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum SceneId {
    Terminal,
    GradientAscii,
    Scope,
}

impl SceneId {
    /// In the order of the sidebar buttons and the number keys.
    pub const ALL: [SceneId; 3] = [SceneId::Terminal, SceneId::GradientAscii, SceneId::Scope];

    pub fn name(self) -> &'static str {
        match self {
            SceneId::Terminal => "Faulty Terminal",
            SceneId::GradientAscii => "Gradient ASCII",
            SceneId::Scope => "Phosphor Scope",
        }
    }

    /// Creates the scene's GPU side, with targets at `size` (the scene's render size).
    pub fn build(self, gpu: &GpuCtx, size: (u32, u32)) -> Box<dyn Scene> {
        match self {
            SceneId::Terminal => Box::new(terminal::Terminal::new(gpu)),
            SceneId::GradientAscii => Box::new(ascii::GradientAscii::new(gpu)),
            SceneId::Scope => Box::new(scope::Scope::new(gpu, size)),
        }
    }

    /// Position in `ALL`.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The scene after this one, wrapping around.
    pub fn next(self) -> SceneId {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }
}

/// Every scene's own settings, stored in settings.json under `scenes`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSettings {
    pub ascii: ascii::Settings,
    pub scope: scope::Settings,
}

impl SceneSettings {
    /// `id`'s settings, or `None` for a scene without any.
    pub fn config(&mut self, id: SceneId) -> Option<&mut dyn SceneConfig> {
        match id {
            SceneId::Terminal => None,
            SceneId::GradientAscii => Some(&mut self.ascii),
            SceneId::Scope => Some(&mut self.scope),
        }
    }

    /// A random look for every scene.
    pub fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        for id in SceneId::ALL {
            if let Some(c) = self.config(id) {
                c.randomize(r);
            }
        }
    }
}

/// A scene's own settings: what the Visuals drawer shows for it, and Randomize.
pub trait SceneConfig {
    /// The scene's controls in the Visuals drawer (use `ui::widgets`, so each one can be reset).
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens);
    /// A random but watchable look. `r(lo, hi)` is uniform in `lo..hi`.
    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32);
}

/// What a scene draws with this frame.
pub struct SceneFrame<'a> {
    /// The HDR scene target (`gpu::HDR_FORMAT`, at the scene's render size); bloom and tonemap
    /// read it afterwards.
    pub target: &'a wgpu::TextureView,
    /// Group 0: globals, audio, sampler and this scene's `Params`.
    pub g0: &'a wgpu::BindGroup,
    /// First frame since the scene came on screen (e.g. to drop a stale feedback buffer).
    pub fresh: bool,
}

/// The GPU side of a scene. Every scene is built at startup; only the one on screen is asked
/// for params and drawn.
pub trait Scene: Send {
    /// The scene render size changed. Scenes with their own size-dependent textures rebuild here.
    fn resize(&mut self, _gpu: &GpuCtx, _size: (u32, u32)) {}
    /// Fills this frame's `P` uniform. Not called for scenes that aren't on screen.
    fn params(&mut self, _vis: &VisState, _settings: &SceneSettings, _p: &mut Params) {}
    /// Draws the scene into `f.target`.
    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame);
}

/// Size of the `P` uniform buffer, shared by all scenes.
pub const PARAMS_BYTES: usize = 256;

/// One frame's `P` uniform (group 0, binding 3) for the scene on screen.
pub struct Params([u8; PARAMS_BYTES]);

impl Default for Params {
    fn default() -> Self {
        Self([0; PARAMS_BYTES])
    }
}

impl Params {
    /// Stores `v`, a `#[repr(C)]` struct laid out like the shader's `Params`.
    pub fn set<T: Pod>(&mut self, v: &T) {
        let b = bytemuck::bytes_of(v);
        assert!(b.len() <= PARAMS_BYTES, "scene params are {} bytes, max {PARAMS_BYTES}", b.len());
        self.0[..b.len()].copy_from_slice(b);
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}
