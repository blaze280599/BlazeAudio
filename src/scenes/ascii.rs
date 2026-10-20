//! Gradient ASCII: a soft light seen through a grid of glyphs sampled from a bundled-font atlas.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use super::{Params, Scene, SceneConfig, SceneFrame, SceneSettings};
use crate::gpu::{self, GpuCtx, HDR_FORMAT};
use crate::palette::{linear_to_oklab, luminance};
use crate::ui::widgets::{choice, knob, num, PCT};
use crate::ui::Tokens;
use crate::vis::VisState;

/// Characters, dimmest to brightest.
const CHARS: &str = " =+xX0S8#@";
const ATLAS_CELL_W: u32 = 60;
const ATLAS_CELL_H: u32 = 100;

/// Shape of the light (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Shape {
    Horizon,
    Pinch,
    Rise,
    Eclipse,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub shape: Shape,
    /// Cell height in pixels at 1080p.
    pub cell: f32,
    pub density: f32,
    pub glyphs: f32,
    pub gradient: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { shape: Shape::Horizon, cell: 18.0, density: 0.8, glyphs: 0.85, gradient: 0.9 }
    }
}

impl SceneConfig for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let d = Self::default();
        let shapes = [(Shape::Horizon, "Horizon"), (Shape::Pinch, "Pinch"), (Shape::Rise, "Rise"), (Shape::Eclipse, "Eclipse")];
        choice(ui, t, "Shape", &mut self.shape, d.shape, &shapes);
        knob(ui, t, "Cell size", &mut self.cell, d.cell, 8.0..=48.0, num(0, " px"));
        knob(ui, t, "Density", &mut self.density, d.density, 0.0..=1.0, PCT);
        knob(ui, t, "Glyph opacity", &mut self.glyphs, d.glyphs, 0.0..=1.0, PCT);
        knob(ui, t, "Gradient opacity", &mut self.gradient, d.gradient, 0.0..=1.0, PCT);
    }

    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        self.shape = [Shape::Horizon, Shape::Pinch, Shape::Rise, Shape::Eclipse][(r(0.0, 4.0) as usize).min(3)];
        self.cell = r(10.0, 32.0).round();
        self.density = r(0.5, 1.0);
        self.glyphs = r(0.6, 1.0);
        self.gradient = r(0.6, 1.0);
    }
}

/// Must match `Params` in ascii.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AsciiParams {
    shape: f32,
    cell: f32,
    density: f32,
    glyphs: f32,
    gradient: f32,
    count: f32,
    _pad: [f32; 2],
    /// The palette in Oklab, sorted from the edge of the shadow (darkest) to the brightest point.
    pal_lab: [[f32; 4]; 4],
}

pub struct GradientAscii {
    pipe: wgpu::RenderPipeline,
    /// Owns the texture behind `bg_atlas`.
    _atlas: wgpu::Texture,
    bg_atlas: wgpu::BindGroup,
}

impl GradientAscii {
    pub fn new(gpu: &GpuCtx) -> Self {
        let m = gpu.module("ascii", include_str!("../shaders/ascii.wgsl"));
        let pipe = gpu.fullscreen(&gpu.layout(true), &m, "fs_ascii", HDR_FORMAT, None);
        let atlas = build_atlas(gpu.device, gpu.queue);
        let bg_atlas = gpu.tex1(&atlas.create_view(&wgpu::TextureViewDescriptor::default()));
        Self { pipe, _atlas: atlas, bg_atlas }
    }
}

impl Scene for GradientAscii {
    fn params(&mut self, vis: &VisState, settings: &SceneSettings, p: &mut Params) {
        let s = &settings.ascii;
        let mut ramp = vis.palette();
        ramp.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
        let mut pal_lab = [[0.0f32; 4]; 4];
        for i in 0..4 {
            let l = linear_to_oklab(ramp[i]);
            pal_lab[i] = [l[0], l[1], l[2], 0.0];
        }
        p.set(&AsciiParams {
            shape: s.shape as u32 as f32,
            cell: s.cell,
            density: s.density,
            glyphs: s.glyphs,
            gradient: s.gradient,
            count: CHARS.chars().count() as f32,
            _pad: [0.0; 2],
            pal_lab,
        });
    }

    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        gpu::fullscreen_pass(enc, f.target, wgpu::LoadOp::Clear(wgpu::Color::BLACK), &self.pipe, f.g0, Some(&self.bg_atlas));
    }
}

/// Rasterize `CHARS` (Hack, from egui's bundled fonts) into one row of 60x100 cells, coverage
/// in R, with a full mip chain so small cells stay smooth.
fn build_atlas(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

    let chars: Vec<char> = CHARS.chars().collect();
    let (w, h) = (ATLAS_CELL_W * chars.len() as u32, ATLAS_CELL_H);
    let mut px = vec![0u8; (w * h) as usize];
    let font = FontRef::try_from_slice(epaint_default_fonts::HACK_REGULAR).expect("bundled font");
    let sf = font.as_scaled(PxScale::from(ATLAS_CELL_H as f32 * 0.78));
    // Centre the em box vertically, like canvas `textBaseline = middle`.
    let baseline = ATLAS_CELL_H as f32 / 2.0 + (sf.ascent() + sf.descent()) / 2.0;
    for (i, &ch) in chars.iter().enumerate() {
        let id = font.glyph_id(ch);
        let x0 = (i as u32 * ATLAS_CELL_W) as f32 + (ATLAS_CELL_W as f32 - sf.h_advance(id)) / 2.0;
        let glyph = id.with_scale_and_position(sf.scale(), ab_glyph::point(x0, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, c| {
                let (x, y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                    let p = &mut px[(y as u32 * w + x as u32) as usize];
                    *p = (*p).max((c.clamp(0.0, 1.0) * 255.0) as u8);
                }
            });
        }
    }

    let mips = 32 - w.max(h).leading_zeros();
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ascii atlas"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let base = image::GrayImage::from_raw(w, h, px).expect("atlas size");
    for level in 0..mips {
        let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
        let img = if level == 0 { base.clone() } else { image::imageops::resize(&base, lw, lh, image::imageops::FilterType::Triangle) };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            img.as_raw(),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw), rows_per_image: None },
            wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
        );
    }
    tex
}
