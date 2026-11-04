//! Picture Disc: late-night music TV. A CD printed with the album cover spins at the left edge
//! while a "now playing." card shows the cover, title, artist and album over a segmented spectrum
//! meter, and the whole picture goes through a VHS pass. The card's text is rasterized into an
//! atlas when the track changes, never per frame.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;

use ab_glyph::{Font, FontArc, FontVec, GlyphId, PxScale, ScaleFont};
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use super::{Params, Scene, SceneConfig, SceneFrame, SceneSettings};
use crate::gpu::{self, GpuCtx, Tex, HDR_FORMAT};
use crate::media::{TrackInfo, ART_SIZE};
use crate::palette::{linear_to_srgb, srgb_to_linear};
use crate::ui::widgets::{choice, knob, num, PCT};
use crate::ui::Tokens;
use crate::vis::VisState;

/// Text atlas: one line of text per row, `ROWS` rows (see `Row`), with mips down to 4 px rows.
const ROW_H: u32 = 128;
const ATLAS_W: u32 = 2048;
const ROWS: usize = 5;
const ATLAS_H: u32 = ROW_H * ROWS as u32;
const ATLAS_MIPS: u32 = 6;
/// Em size, baseline and left margin of the text in a row, in atlas pixels.
const EM: f32 = 92.0;
const BASELINE: f32 = 100.0;
const PAD: f32 = 8.0;
/// Height of the giant background word and the gap between its repeats, in scene heights.
const GHOST_H: f32 = 0.62;
const GHOST_GAP: f32 = 0.35;
/// Album art mips, down to 1x1: the Album backdrop is the cover blurred by sampling a small one.
const ART_MIPS: u32 = ART_SIZE.ilog2() + 1;

/// Atlas rows (order matches the shader).
#[derive(Clone, Copy)]
enum Row {
    NowPlaying,
    Title,
    Artist,
    Album,
    Ghost,
}

impl Row {
    /// After the reference: a plain "now playing.", italic track lines and a heavy ghost word.
    fn style(self) -> Style {
        match self {
            Row::NowPlaying => Style::Regular,
            Row::Title | Row::Artist | Row::Album => Style::Italic,
            Row::Ghost => Style::Bold,
        }
    }

    /// Longest line the card has room for, in atlas pixels; longer text ends in "...".
    fn max_width(self) -> f32 {
        match self {
            Row::Title | Row::Artist => 1100.0,
            Row::Album => 1360.0,
            Row::NowPlaying | Row::Ghost => ATLAS_W as f32,
        }
    }
}

/// Weight and style of an atlas row.
#[derive(Clone, Copy, PartialEq)]
enum Style {
    Regular,
    Italic,
    Bold,
}

impl Style {
    /// Windows fonts in order of preference: Arial for the look, then fonts for Japanese, Korean
    /// and Chinese titles. Each is read only when a line needs it.
    fn system_fonts(self) -> [&'static str; 5] {
        match self {
            Style::Regular => ["arial.ttf", "segoeui.ttf", "YuGothR.ttc", "malgun.ttf", "msyh.ttc"],
            Style::Italic => ["ariali.ttf", "segoeui.ttf", "YuGothR.ttc", "malgun.ttf", "msyh.ttc"],
            Style::Bold => ["arialbd.ttf", "segoeuib.ttf", "YuGothB.ttc", "malgunbd.ttf", "msyhbd.ttc"],
        }
    }
}

/// Slant drawn into italic lines set in a fallback font with no italic, matching Arial Italic.
const FAUX_SLANT: f32 = 0.21;

/// What's printed on the disc (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Face {
    Cover,
    Chrome,
}

/// What's behind the disc and the card (order matches the shader).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Backdrop {
    Album,
    Black,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub face: Face,
    /// Disc diameter as a fraction of the scene height.
    pub size: f32,
    /// Revolutions per minute before the music speeds it up.
    pub spin: f32,
    /// How much energy and beats speed up the spin and kick the picture.
    pub reactivity: f32,
    pub backdrop: Backdrop,
    /// Opacity of the giant artist name behind everything.
    pub ghost: f32,
    /// Strength of the VHS tape look.
    pub vhs: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { face: Face::Cover, size: 0.94, spin: 18.0, reactivity: 0.5, backdrop: Backdrop::Album, ghost: 0.5, vhs: 0.7 }
    }
}

impl SceneConfig for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let d = Self::default();
        choice(ui, t, "Disc", &mut self.face, d.face, &[(Face::Cover, "Cover"), (Face::Chrome, "Chrome")]);
        knob(ui, t, "Size", &mut self.size, d.size, 0.5..=1.3, PCT);
        knob(ui, t, "Spin", &mut self.spin, d.spin, 0.0..=90.0, num(0, " rpm"));
        knob(ui, t, "Reactivity", &mut self.reactivity, d.reactivity, 0.0..=1.0, PCT);
        choice(ui, t, "Backdrop", &mut self.backdrop, d.backdrop, &[(Backdrop::Album, "Album"), (Backdrop::Black, "Black")]);
        knob(ui, t, "Ghost text", &mut self.ghost, d.ghost, 0.0..=1.0, PCT);
        knob(ui, t, "VHS", &mut self.vhs, d.vhs, 0.0..=1.0, PCT);
    }

    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        self.face = if r(0.0, 1.0) < 0.7 { Face::Cover } else { Face::Chrome };
        self.size = r(0.8, 1.1);
        self.spin = r(8.0, 40.0).round();
        self.reactivity = r(0.2, 0.9);
        self.backdrop = if r(0.0, 1.0) < 0.75 { Backdrop::Album } else { Backdrop::Black };
        self.ghost = r(0.2, 0.8);
        self.vhs = r(0.3, 1.0);
    }
}

/// Must match `Params` in disc.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DiscParams {
    angle: f32,
    radius: f32,
    face: f32,
    backdrop: f32,
    ghost: f32,
    vhs: f32,
    has_art: f32,
    /// Seconds since the card's text changed.
    info_t: f32,
    /// Width of each atlas row's text, in row heights.
    widths: [f32; 4],
    ghost_w: f32,
    ghost_h: f32,
    ghost_period: f32,
    scroll: f32,
}

pub struct PictureDisc {
    disc: wgpu::RenderPipeline,
    vhs: wgpu::RenderPipeline,
    art: wgpu::Texture,
    text: wgpu::Texture,
    /// Album art and the text atlas, for fs_disc.
    bg_src: wgpu::BindGroup,
    /// fs_disc draws here; fs_vhs reads it into the scene target.
    comp: Tex,
    bg_comp: wgpu::BindGroup,
    has_art: bool,
    /// The cover last uploaded, so the same one isn't uploaded twice.
    art_src: Option<Arc<Vec<u8>>>,
    /// Title, artist and album lines last rasterized.
    lines: Option<[String; 3]>,
    widths: [f32; ROWS],
    angle: f32,
    scroll: f32,
    info_t: f32,
}

impl PictureDisc {
    pub fn new(gpu: &GpuCtx, size: (u32, u32)) -> Self {
        let m = gpu.module("disc", include_str!("../shaders/disc.wgsl"));
        let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&gpu.layouts.g0, &gpu.layouts.tex2],
            push_constant_ranges: &[],
        });
        let texture = |w: u32, h: u32, mips: u32, format: wgpu::TextureFormat, label: &str| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let art = texture(ART_SIZE, ART_SIZE, ART_MIPS, wgpu::TextureFormat::Rgba8UnormSrgb, "disc art");
        let text = texture(ATLAS_W, ATLAS_H, ATLAS_MIPS, wgpu::TextureFormat::R8Unorm, "disc text");
        let bg_src = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &gpu.layouts.tex2,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&art.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&text.create_view(&Default::default())) },
            ],
        });
        let comp = gpu::make_tex(gpu.device, size.0, size.1, HDR_FORMAT, "disc comp");
        let bg_comp = gpu.tex1(&comp.view);
        let mut s = Self {
            disc: gpu.fullscreen(&layout, &m, "fs_disc", HDR_FORMAT, None),
            vhs: gpu.fullscreen(&gpu.layout(true), &m, "fs_vhs", HDR_FORMAT, None),
            art,
            text,
            bg_src,
            comp,
            bg_comp,
            has_art: false,
            art_src: None,
            lines: None,
            widths: [0.0; ROWS],
            angle: 0.0,
            scroll: 0.0,
            info_t: 0.0,
        };
        s.track(gpu, &TrackInfo::default());
        s
    }

    /// Rasterizes the card's lines into the atlas, unless they're what's already there.
    fn set_text(&mut self, queue: &wgpu::Queue, track: &TrackInfo) {
        let (title, artist, album) = (track.title.trim(), track.artist.trim(), track.album.trim());
        let dash = |s: &str| if s.is_empty() { String::new() } else { format!("- {s}") };
        let lines = if title.is_empty() {
            ["no signal".to_owned(), "- press play".to_owned(), String::new()]
        } else {
            [title.to_owned(), dash(artist), dash(album)]
        };
        if self.lines.as_ref() == Some(&lines) {
            return;
        }
        let ghost = if !artist.is_empty() { artist } else if !title.is_empty() { title } else { "blazeaudio" }.to_lowercase();
        let rows = [
            (Row::NowPlaying, "now playing."),
            (Row::Title, lines[0].as_str()),
            (Row::Artist, lines[1].as_str()),
            (Row::Album, lines[2].as_str()),
            (Row::Ghost, ghost.as_str()),
        ];
        let mut fonts = [Fonts::new(Style::Regular), Fonts::new(Style::Italic), Fonts::new(Style::Bold)];
        let mut px = vec![0u8; (ATLAS_W * ATLAS_H) as usize];
        for (row, text) in rows {
            let fonts = &mut fonts[row.style() as usize];
            let (glyphs, end) = fit(fonts, text, row.max_width());
            fonts.draw(&glyphs, &mut px, row as u32);
            self.widths[row as usize] = if glyphs.is_empty() { 0.0 } else { (end + PAD) / ROW_H as f32 };
        }
        let (mut w, mut h) = (ATLAS_W, ATLAS_H);
        for level in 0..ATLAS_MIPS {
            if level > 0 {
                px = half(&px, w, h);
                (w, h) = (w / 2, h / 2);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.text, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &px,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: None },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        self.lines = Some(lines);
        self.info_t = 0.0;
    }
}

impl Scene for PictureDisc {
    fn resize(&mut self, gpu: &GpuCtx, size: (u32, u32)) {
        self.comp = gpu::make_tex(gpu.device, size.0, size.1, HDR_FORMAT, "disc comp");
        self.bg_comp = gpu.tex1(&self.comp.view);
    }

    fn track(&mut self, gpu: &GpuCtx, track: &TrackInfo) {
        self.set_text(gpu.queue, track);
        match &track.art {
            Some(a) if a.len() == (ART_SIZE * ART_SIZE * 4) as usize => {
                if !self.art_src.as_ref().is_some_and(|s| Arc::ptr_eq(s, a)) {
                    upload_art(gpu.queue, &self.art, a);
                    self.art_src = Some(a.clone());
                }
                self.has_art = true;
            }
            _ => {
                self.has_art = false;
                self.art_src = None;
            }
        }
    }

    fn params(&mut self, vis: &VisState, settings: &SceneSettings, p: &mut Params) {
        let s = &settings.disc;
        let dt = vis.dt;
        // Energy and beats surge the spin, like a hand pushing the platter.
        let rate = s.spin / 60.0 * TAU * (1.0 + s.reactivity * (vis.energy * 1.2 + vis.kick * 2.0));
        self.angle = (self.angle + rate * dt) % TAU;
        let ghost_w = self.widths[Row::Ghost as usize];
        let period = ghost_w * GHOST_H + GHOST_GAP;
        self.scroll = (self.scroll + dt * (0.015 + 0.04 * vis.energy)) % period;
        self.info_t = (self.info_t + dt).min(100.0);
        p.set(&DiscParams {
            angle: self.angle,
            radius: s.size * 0.5,
            face: s.face as u32 as f32,
            backdrop: s.backdrop as u32 as f32,
            ghost: s.ghost,
            vhs: s.vhs,
            has_art: if self.has_art { 1.0 } else { 0.0 },
            info_t: self.info_t,
            widths: [self.widths[0], self.widths[1], self.widths[2], self.widths[3]],
            ghost_w,
            ghost_h: GHOST_H,
            ghost_period: period,
            scroll: self.scroll,
        });
    }

    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        let black = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        gpu::fullscreen_pass(enc, &self.comp.view, black, &self.disc, f.g0, Some(&self.bg_src));
        gpu::fullscreen_pass(enc, f.target, black, &self.vhs, f.g0, Some(&self.bg_comp));
    }
}

/// Uploads a cover (sRGB RGBA8, `ART_SIZE` square) with its mips, averaged in linear light so
/// the blurred backdrop keeps the cover's real brightness.
fn upload_art(queue: &wgpu::Queue, tex: &wgpu::Texture, rgba: &[u8]) {
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let mut lin: Vec<f32> = rgba.iter().enumerate().map(|(i, &v)| if i % 4 == 3 { v as f32 / 255.0 } else { lut[v as usize] }).collect();
    let mut size = ART_SIZE;
    for level in 0..ART_MIPS {
        if level > 0 {
            let (n, h) = (size as usize, size as usize / 2);
            let mut next = vec![0.0f32; h * h * 4];
            for y in 0..h {
                for x in 0..h {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| lin[((2 * y + dy) * n + 2 * x + dx) * 4 + c];
                        next[(y * h + x) * 4 + c] = (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25;
                    }
                }
            }
            lin = next;
            size /= 2;
        }
        let px: Vec<u8> = if level == 0 {
            rgba.to_vec()
        } else {
            lin.iter().enumerate().map(|(i, &v)| ((if i % 4 == 3 { v } else { linear_to_srgb(v) }).clamp(0.0, 1.0) * 255.0).round() as u8).collect()
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: None },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        );
    }
}

/// The fonts of one style loaded so far for one rasterization; see `Style::system_fonts`.
/// Dropped afterwards, so the large CJK fonts don't stay in memory.
struct Fonts {
    style: Style,
    loaded: Vec<FontArc>,
    /// Slant to draw each loaded font with (`FAUX_SLANT` for italic lines in an upright font).
    slant: Vec<f32>,
    /// Next system font to try; one past the end means the bundled font is next.
    next: usize,
}

impl Fonts {
    fn new(style: Style) -> Self {
        Self { style, loaded: Vec::new(), slant: Vec::new(), next: 0 }
    }

    /// Loads the next font in line, or returns false when there are none left.
    fn load_next(&mut self) -> bool {
        let dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts");
        let names = self.style.system_fonts();
        // Only the first italic choice (Arial Italic) is a real italic.
        let faux = |next: usize| if self.style == Style::Italic && next > 0 { FAUX_SLANT } else { 0.0 };
        while let Some(name) = names.get(self.next) {
            let slant = faux(self.next);
            self.next += 1;
            if let Some(f) = std::fs::read(dir.join(name)).ok().and_then(|d| FontVec::try_from_vec_and_index(d, 0).ok()) {
                self.loaded.push(FontArc::new(f));
                self.slant.push(slant);
                return true;
            }
        }
        if self.next == names.len() {
            let slant = faux(self.next);
            self.next += 1;
            if let Ok(f) = FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT) {
                self.loaded.push(f);
                self.slant.push(slant);
                return true;
            }
        }
        false
    }

    /// The first font with a glyph for `c`, loading more as needed.
    fn find(&mut self, c: char) -> Option<usize> {
        loop {
            if let Some(i) = self.loaded.iter().position(|f| f.glyph_id(c).0 != 0) {
                return Some(i);
            }
            if !self.load_next() {
                return None;
            }
        }
    }

    /// Draws `glyphs` into atlas row `row` of `px`, clipped to the row.
    fn draw(&self, glyphs: &[Glyph], px: &mut [u8], row: u32) {
        for g in glyphs {
            let (f, slant) = (&self.loaded[g.font], self.slant[g.font]);
            let glyph = g.id.with_scale_and_position(scale(f), ab_glyph::point(g.x, BASELINE));
            let Some(o) = f.outline_glyph(glyph) else { continue };
            let b = o.px_bounds();
            o.draw(|gx, gy, c| {
                let y = b.min.y as i32 + gy as i32;
                let x = b.min.x as i32 + gx as i32 + ((BASELINE - y as f32) * slant).round() as i32;
                if x >= 0 && y >= 0 && (x as u32) < ATLAS_W && (y as u32) < ROW_H {
                    let p = &mut px[((row * ROW_H + y as u32) * ATLAS_W + x as u32) as usize];
                    *p = (*p).max((c.clamp(0.0, 1.0) * 255.0) as u8);
                }
            });
        }
    }
}

struct Glyph {
    font: usize,
    id: GlyphId,
    x: f32,
}

/// Scale giving `f` an em of `EM` pixels, so fallback fonts match Arial's size.
fn scale(f: &FontArc) -> PxScale {
    PxScale::from(EM * f.height_unscaled() / f.units_per_em().unwrap_or(1000.0))
}

/// Emoji and pictographs: no font here has them, and looking for one would read every fallback.
fn is_pictograph(c: char) -> bool {
    matches!(c as u32, 0x2600..=0x27BF | 0x1F000..=0x1FAFF | 0xFE00..=0xFE0F | 0x200D)
}

/// `text` on one line starting at `PAD`, and where the line ends.
fn layout(fonts: &mut Fonts, text: &str) -> (Vec<Glyph>, f32) {
    let mut out = Vec::new();
    let mut x = PAD;
    let mut prev: Option<(usize, GlyphId)> = None;
    for c in text.chars() {
        if c.is_control() || is_pictograph(c) {
            continue;
        }
        let Some(fi) = fonts.find(c) else { continue };
        let f = &fonts.loaded[fi];
        let sf = f.as_scaled(scale(f));
        let id = f.glyph_id(c);
        if let Some((_, pid)) = prev.filter(|p| p.0 == fi) {
            x += sf.kern(pid, id);
        }
        out.push(Glyph { font: fi, id, x });
        x += sf.h_advance(id);
        prev = Some((fi, id));
    }
    (out, x)
}

/// `text` laid out within `max` pixels, cut short with "..." if it doesn't fit.
fn fit(fonts: &mut Fonts, text: &str, max: f32) -> (Vec<Glyph>, f32) {
    let full = layout(fonts, text);
    if full.1 + PAD <= max {
        return full;
    }
    let chars: Vec<char> = text.chars().collect();
    for n in (0..chars.len()).rev() {
        let cut: String = chars[..n].iter().collect();
        let line = layout(fonts, &format!("{}...", cut.trim_end()));
        if line.1 + PAD <= max {
            return line;
        }
    }
    layout(fonts, "...")
}

/// Half-size copy of a `w` x `h` coverage image (2x2 box filter), for the atlas mips.
fn half(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (hw, hh) = (w / 2, h / 2);
    let mut out = vec![0u8; (hw * hh) as usize];
    for y in 0..hh {
        let (r0, r1) = (&src[(2 * y * w) as usize..], &src[((2 * y + 1) * w) as usize..]);
        for x in 0..hw {
            let i = (2 * x) as usize;
            let sum = r0[i] as u32 + r0[i + 1] as u32 + r1[i] as u32 + r1[i + 1] as u32;
            out[(y * hw + x) as usize] = ((sum + 2) / 4) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_averages_blocks() {
        let src = [0, 255, 10, 10, 255, 0, 10, 10];
        assert_eq!(half(&src, 4, 2), vec![128, 10]);
    }

    #[test]
    fn long_lines_are_cut_with_an_ellipsis() {
        let mut fonts = Fonts::new(Style::Italic);
        let (_, short) = fit(&mut fonts, "short", 1100.0);
        let (glyphs, long) = fit(&mut fonts, &"very long title ".repeat(20), 1100.0);
        assert!(short < 1100.0 && long + PAD <= 1100.0 && long > 800.0);
        let dot = fonts.loaded[glyphs[glyphs.len() - 1].font].glyph_id('.');
        assert!(glyphs[glyphs.len() - 3..].iter().all(|g| g.id == dot));
    }
}
