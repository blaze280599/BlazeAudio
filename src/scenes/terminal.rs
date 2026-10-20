//! Faulty Terminal: one full-screen pass, driven by the globals alone.

use super::{Scene, SceneFrame};
use crate::gpu::{self, GpuCtx, HDR_FORMAT};

pub struct Terminal {
    pipe: wgpu::RenderPipeline,
}

impl Terminal {
    pub fn new(gpu: &GpuCtx) -> Self {
        let m = gpu.module("terminal", include_str!("../shaders/terminal.wgsl"));
        Self { pipe: gpu.fullscreen(&gpu.layout(false), &m, "fs_terminal", HDR_FORMAT, None) }
    }
}

impl Scene for Terminal {
    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        gpu::fullscreen_pass(enc, f.target, wgpu::LoadOp::Clear(wgpu::Color::BLACK), &self.pipe, f.g0, None);
    }
}
