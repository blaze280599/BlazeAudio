#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod gpu;
mod media;
mod palette;
mod settings;
mod snapshot;
mod sys;
mod ui;
mod vis;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Fullscreen, Icon, Window, WindowId, WindowLevel};

use audio::{AudioEngine, AudioParams};
use gpu::{FrameDesc, Gpu, MAX_PARTICLES, SCENE_COUNT, SCENE_NEBULA};
use media::{MediaWatcher, TrackInfo};
use palette::PRESETS;
use settings::{Settings, WindowMode};
use vis::{SceneMgr, VisState};

pub fn make_instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: Default::default(),
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions {
                shader_compiler: wgpu::Dx12Compiler::Fxc,
                // A DirectComposition visual swapchain supports premultiplied alpha, which is
                // what makes the transparent desktop overlay possible.
                presentation_system: wgpu::wgt::Dx12SwapchainKind::DxgiFromVisual,
                latency_waitable_object: Default::default(),
            },
            ..Default::default()
        },
    })
}

pub fn audio_params(s: &Settings) -> AudioParams {
    AudioParams {
        attack_ms: s.attack_ms,
        decay_ms: s.decay_ms,
        sensitivity: s.sensitivity,
        beat_sensitivity: s.beat_sensitivity,
    }
}

fn fatal(msg: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe {
        MessageBoxW(None, &HSTRING::from(msg), &HSTRING::from("BlazeAudio"), MB_OK | MB_ICONERROR);
    }
}

fn app_icon() -> Option<Icon> {
    const N: u32 = 64;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let dx = x as f32 - 31.5;
            let dy = y as f32 - 31.5;
            let r = (dx * dx + dy * dy).sqrt() / 31.5;
            let a = dy.atan2(dx) / std::f32::consts::TAU + 0.5;
            let ring = (1.0 - ((r - 0.72).abs() / 0.16)).clamp(0.0, 1.0);
            let core = (1.0 - r / 0.4).clamp(0.0, 1.0);
            let bar = if ((a * 24.0).fract() - 0.5).abs() < 0.25 && r > 0.5 && r < 0.95 { 1.0 } else { 0.0 };
            let v = (ring.powf(0.6) + bar * 0.6 + core * 0.8).min(1.0);
            let t = a;
            let (cr, cg, cb) = (1.0 - t * 0.9, 0.25 + t * 0.5, 0.6 + t * 0.4);
            px.extend_from_slice(&[(cr * v * 255.0) as u8, (cg * v * 255.0) as u8, (cb * v * 255.0) as u8, (v * 255.0) as u8]);
        }
    }
    Icon::from_rgba(px, N, N).ok()
}

struct Gfx {
    window: Arc<Window>,
    gpu: Gpu,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
}

struct App {
    settings: Settings,
    instance: wgpu::Instance,
    audio: AudioEngine,
    media: MediaWatcher,
    vis: VisState,
    scenes: SceneMgr,
    gfx: Option<Gfx>,
    ui: ui::UiState,

    mode: WindowMode,
    windowed_rect: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
    show_sidebar: bool,
    modifiers: ModifiersState,

    start: Instant,
    last_frame: Instant,
    next_frame: Instant,
    fps: f32,
    frame_ms: f32,
    idle: bool,
    occluded: bool,
    minimized: bool,

    track: TrackInfo,
    track_ver: u64,
    art_ver: u64,
    art_present: f32,
    toast_t: f32,

    hotkeys: sys::GlobalHotkeys,
    stats: sys::ProcStats,
    trim_at: Option<Instant>,
    last_mouse_move: Instant,
    cursor_hidden: bool,
    last_click: Instant,
    last_beat_seen: bool,
    hittest: Option<bool>,
    ignore_audio: bool,
    /// Scene area in physical pixels (x, y, w, h), i.e. the window minus docked panels.
    scene_px: [f32; 4],
    cursor: PhysicalPosition<f64>,
}

impl App {
    fn new() -> Self {
        let settings = Settings::load();
        let audio = AudioEngine::start(audio_params(&settings));
        let media = MediaWatcher::start();
        let pal = PRESETS[settings.preset_palette.min(PRESETS.len() - 1)].1;
        let now = Instant::now();
        Self {
            scenes: SceneMgr::new(settings.scene),
            mode: WindowMode::Windowed,
            instance: make_instance(),
            audio,
            media,
            vis: VisState::new(pal),
            gfx: None,
            ui: ui::UiState::new(),
            windowed_rect: None,
            show_sidebar: settings.start_mode == WindowMode::Windowed,
            modifiers: ModifiersState::empty(),
            start: now,
            last_frame: now,
            next_frame: now,
            fps: 0.0,
            frame_ms: 0.0,
            idle: false,
            occluded: false,
            minimized: false,
            track: TrackInfo::default(),
            track_ver: 0,
            art_ver: 0,
            art_present: 0.0,
            toast_t: 0.0,
            hotkeys: sys::GlobalHotkeys::new(),
            stats: sys::ProcStats::new(),
            trim_at: Some(now + Duration::from_secs(4)),
            last_mouse_move: now,
            cursor_hidden: false,
            last_click: now - Duration::from_secs(5),
            last_beat_seen: false,
            hittest: None,
            ignore_audio: std::env::var_os("BLAZE_IGNORE_AUDIO").is_some(),
            scene_px: [0.0, 0.0, f32::MAX, f32::MAX],
            cursor: PhysicalPosition::new(0.0, 0.0),
            settings,
        }
    }

    fn switch_scene(&mut self, n: usize) {
        self.scenes.request(n);
        if self.scenes.cur == SCENE_NEBULA && self.scenes.prev != Some(SCENE_NEBULA) {
            if let Some(g) = &mut self.gfx {
                g.gpu.reset_trails();
            }
        }
        self.settings.scene = self.scenes.cur;
    }

    fn set_sidebar_visible(&mut self, v: bool) {
        if self.show_sidebar && !v {
            self.ui.drawer = None;
            self.settings.save();
            self.trim_at = Some(Instant::now() + Duration::from_secs(2));
        }
        self.show_sidebar = v;
        self.update_hittest();
        if v {
            if let Some(g) = &self.gfx {
                g.window.focus_window();
                g.window.set_cursor_visible(true);
                self.cursor_hidden = false;
            }
        }
    }

    fn update_hittest(&mut self) {
        if let Some(g) = &self.gfx {
            let interactive = self.mode != WindowMode::Overlay || self.show_sidebar || !self.settings.overlay_click_through;
            if self.hittest != Some(interactive) {
                self.hittest = Some(interactive);
                let _ = g.window.set_cursor_hittest(interactive);
                sys::fix_layered(&g.window);
            }
        }
    }

    fn apply_mode(&mut self, mode: WindowMode) {
        let Some(g) = &self.gfx else { return };
        let w = g.window.clone();
        if self.mode == WindowMode::Windowed && mode != WindowMode::Windowed {
            if let Ok(pos) = w.outer_position() {
                self.windowed_rect = Some((pos, w.inner_size()));
            }
        }
        match mode {
            WindowMode::Windowed => {
                w.set_fullscreen(None);
                w.set_window_level(WindowLevel::Normal);
                w.set_decorations(true);
                if let Some((pos, size)) = self.windowed_rect {
                    w.set_outer_position(pos);
                    let _ = w.request_inner_size(size);
                }
            }
            WindowMode::Fullscreen => {
                w.set_window_level(WindowLevel::Normal);
                w.set_decorations(true);
                w.set_fullscreen(Some(Fullscreen::Borderless(None)));
            }
            WindowMode::Overlay => {
                w.set_fullscreen(None);
                w.set_decorations(false);
                if let Some(m) = w.current_monitor() {
                    w.set_outer_position(m.position());
                    let _ = w.request_inner_size(m.size());
                }
                w.set_window_level(WindowLevel::AlwaysOnTop);
            }
        }
        self.mode = mode;
        // The sidebar is part of the windowed app; fullscreen and overlay are for the visuals.
        self.set_sidebar_visible(mode == WindowMode::Windowed);
        #[cfg(debug_assertions)]
        eprintln!("[win] mode={mode:?} size={:?} pos={:?}", w.inner_size(), w.outer_position());
        w.set_cursor_visible(true);
        self.cursor_hidden = false;
        self.update_hittest();
        self.trim_at = Some(Instant::now() + Duration::from_secs(3));
    }

    fn toggle_mode(&mut self, m: WindowMode) {
        let target = if self.mode == m { WindowMode::Windowed } else { m };
        self.apply_mode(target);
    }

    fn handle_key(&mut self, el: &ActiveEventLoop, code: KeyCode) {
        let alt = self.modifiers.alt_key();
        match code {
            KeyCode::F1 | KeyCode::Tab => self.set_sidebar_visible(!self.show_sidebar),
            KeyCode::F11 | KeyCode::KeyF => self.toggle_mode(WindowMode::Fullscreen),
            KeyCode::Enter if alt => self.toggle_mode(WindowMode::Fullscreen),
            KeyCode::KeyO => self.toggle_mode(WindowMode::Overlay),
            KeyCode::ArrowRight | KeyCode::KeyN | KeyCode::Space => self.switch_scene(self.scenes.cur + 1),
            KeyCode::ArrowLeft | KeyCode::KeyP => self.switch_scene(self.scenes.cur + SCENE_COUNT - 1),
            KeyCode::Digit1 => self.switch_scene(0),
            KeyCode::Digit2 => self.switch_scene(1),
            KeyCode::Digit3 => self.switch_scene(2),
            KeyCode::Digit4 => self.switch_scene(3),
            KeyCode::Digit5 => self.switch_scene(4),
            KeyCode::Digit6 => self.switch_scene(5),
            KeyCode::Digit7 => self.switch_scene(6),
            KeyCode::Digit8 => self.switch_scene(7),
            KeyCode::Digit9 => self.switch_scene(8),
            KeyCode::Digit0 => self.switch_scene(9),
            KeyCode::Minus => self.switch_scene(10),
            KeyCode::KeyT => self.toast_t = 6.0,
            KeyCode::F3 => self.settings.show_stats = !self.settings.show_stats,
            KeyCode::Escape => {
                if self.ui.drawer.is_some() {
                    self.ui.drawer = None;
                    self.settings.save();
                } else if self.show_sidebar && self.mode != WindowMode::Windowed {
                    self.set_sidebar_visible(false);
                } else if self.mode != WindowMode::Windowed {
                    self.apply_mode(WindowMode::Windowed);
                } else {
                    self.settings.save();
                    el.exit();
                }
            }
            _ => {}
        }
    }

    fn poll_global_hotkeys(&mut self) {
        match self.hotkeys.poll() {
            Some(sys::GlobalKey::ToggleOverlay) => self.toggle_mode(WindowMode::Overlay),
            Some(sys::GlobalKey::ToggleSettings) => self.set_sidebar_visible(!self.show_sidebar),
            Some(sys::GlobalKey::NextScene) => self.switch_scene(self.scenes.cur + 1),
            None => {}
        }
    }

    fn sync_media(&mut self) {
        let (tv, av) = self.media.versions();
        if tv != self.track_ver || av != self.art_ver {
            let new_track = tv != self.track_ver;
            let new_art = av != self.art_ver;
            self.track_ver = tv;
            self.art_ver = av;
            self.track = self.media.snapshot();
            if new_track && !self.track.title.is_empty() {
                self.toast_t = 6.0;
            }
            if new_art {
                if let Some(g) = &mut self.gfx {
                    if let Some(art) = &self.track.art {
                        g.gpu.upload_art(art);
                    }
                    self.ui.set_art(&g.egui_ctx, self.track.art.as_deref().map(|v| v.as_slice()));
                }
            }
        }
        self.vis.pal_target = match (&self.track.palette, self.settings.album_palette) {
            (Some(p), true) => *p,
            _ => PRESETS[self.settings.preset_palette.min(PRESETS.len() - 1)].1,
        };
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        let Some(mut g) = self.gfx.take() else { return };
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().clamp(1e-4, 0.1);
        self.last_frame = now;
        self.fps += (1.0 / dt - self.fps) * 0.05;
        let time = now.duration_since(self.start).as_secs_f32();

        self.gfx = Some(g);
        self.poll_global_hotkeys();
        self.sync_media();
        g = self.gfx.take().unwrap();

        let art_target = if self.track.art.is_some() { 1.0 } else { 0.0 };
        self.art_present += (art_target - self.art_present) * (1.0 - (-dt / 0.5).exp());
        self.toast_t = (self.toast_t - dt).max(0.0);

        // Audio -> visual state.
        // BLAZE_IGNORE_AUDIO=1 renders as if nothing is playing (for measuring the idle path).
        let frame = if self.ignore_audio { audio::AudioFrame::default() } else { self.audio.snapshot() };
        for (row, data) in self.vis.update(&frame, dt, now) {
            g.gpu.write_spec_row(row, &data);
        }

        // Scene sequencing: auto-cycle lands on a beat; big drops can force a change.
        self.scenes.update(dt, self.settings.transition_seconds);
        let beat = self.vis.beat_this_frame;
        let mut next = None;
        if self.scenes.prev.is_none() {
            let st = self.scenes.scene_time;
            let cyc = self.settings.cycle_seconds;
            if self.settings.auto_cycle && st > cyc && (beat || st > cyc + 4.0) && self.vis.silent_for < 2.0 {
                next = Some(self.scenes.cur + 1);
            } else if self.settings.switch_on_drop && self.vis.drop_this_frame && st > 12.0 {
                next = Some(self.scenes.cur + 1);
            }
        }
        self.last_beat_seen = beat;
        if let Some(n) = next {
            self.gfx = Some(g);
            self.switch_scene(n);
            g = self.gfx.take().unwrap();
        }

        // UI runs first: the docked sidebar decides how much of the window the scene gets.
        // egui only runs when something is on screen, so idle frames cost no UI work.
        let toast_alpha = (self.toast_t / 0.6).min(1.0).min((6.0 - self.toast_t) / 0.4).clamp(0.0, 1.0);
        let still = self.last_mouse_move.elapsed().as_secs_f32();
        let pointer_ok = self.mode != WindowMode::Overlay || self.show_sidebar;
        let controls_alpha = if pointer_ok { (1.0 - (still - 2.0) / 0.5).clamp(0.0, 1.0) } else { 0.0 };
        let need_ui = self.show_sidebar || (self.toast_t > 0.0 && self.settings.show_track_toast) || self.settings.show_stats || controls_alpha > 0.0;
        let drawer_was_open = self.ui.drawer.is_some();
        let mut actions = Vec::new();
        let mut egui_out = None;
        let (sw, sh) = (g.gpu.config.width as f32, g.gpu.config.height as f32);
        self.scene_px = [0.0, 0.0, sw, sh];
        if need_ui {
            let raw = g.egui_state.take_egui_input(&g.window);
            let device_name = self.audio.device_name();
            let palette = self.vis.pal_target;
            let mut scene_rect = egui::Rect::NOTHING;
            let out = g.egui_ctx.run(raw, |ctx| {
                (actions, scene_rect) = self.ui.draw(
                    ctx,
                    ui::UiCtx {
                        settings: &mut self.settings,
                        track: &self.track,
                        vis: &self.vis,
                        palette,
                        scene: self.scenes.cur,
                        mode: self.mode,
                        stats: &self.stats,
                        fps: self.fps,
                        frame_ms: self.frame_ms,
                        idle: self.idle,
                        device: &device_name,
                        adapter: &g.gpu.adapter_name,
                        transparent_ok: g.gpu.transparent_ok,
                        show_sidebar: self.show_sidebar,
                        toast_alpha,
                        controls_alpha,
                    },
                );
            });
            let ppp = out.pixels_per_point;
            let x = (scene_rect.min.x * ppp).round().clamp(0.0, sw);
            let y = (scene_rect.min.y * ppp).round().clamp(0.0, sh);
            self.scene_px = [x, y, (scene_rect.max.x * ppp).round().min(sw) - x, (scene_rect.max.y * ppp).round().min(sh) - y];
            egui_out = Some(out);
        }
        let [vx, vy, vw, vh] = self.scene_px;
        g.gpu.set_viewport(vx as u32, vy as u32, vw.max(0.0) as u32, vh.max(0.0) as u32);

        g.gpu.set_render_scale(self.settings.render_scale);
        let particle_count = (MAX_PARTICLES as f32 * self.settings.particle_fraction) as u32;
        let overlay = self.mode == WindowMode::Overlay && g.gpu.transparent_ok;
        let transition = if self.scenes.prev.is_some() { self.scenes.t } else { 1.0 };
        let globals = self.vis.globals(
            &self.settings,
            g.gpu.scene_size(),
            time,
            dt,
            self.scenes.scene_time,
            transition,
            overlay,
            self.art_present,
            particle_count,
        );
        g.gpu.upload(&globals, &self.vis.audio_buffer());
        g.gpu.upload_splats(&self.vis.splats());

        let surface_tex = match g.gpu.surface.as_ref().unwrap().get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                let size = g.window.inner_size();
                g.gpu.resize(size.width, size.height);
                self.gfx = Some(g);
                return;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                el.exit();
                self.gfx = Some(g);
                return;
            }
            Err(_) => {
                self.gfx = Some(g);
                return;
            }
        };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(g.gpu.view_format),
            ..Default::default()
        });
        let mut enc = g.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let fd = FrameDesc {
            cur_scene: self.scenes.cur,
            cur_slot: self.scenes.cur_slot,
            prev_scene: self.scenes.prev,
            particle_count,
        };
        g.gpu.encode(&mut enc, &view, &fd, overlay);

        if let Some(out) = egui_out {
            g.egui_state.handle_platform_output(&g.window, out.platform_output);
            let prims = g.egui_ctx.tessellate(out.shapes, out.pixels_per_point);
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [g.gpu.config.width, g.gpu.config.height],
                pixels_per_point: out.pixels_per_point,
            };
            g.gpu.encode_egui(&mut enc, &view, &prims, &out.textures_delta, &screen);
        }
        g.gpu.queue.submit([enc.finish()]);
        g.window.pre_present_notify();
        surface_tex.present();
        self.frame_ms += (now.elapsed().as_secs_f32() * 1000.0 - self.frame_ms) * 0.05;
        let drawer_open = self.ui.drawer.is_some();
        if drawer_open {
            self.audio.set_params(audio_params(&self.settings));
        } else if drawer_was_open {
            self.settings.save();
            self.trim_at = Some(Instant::now() + Duration::from_secs(2));
        }

        // Fullscreen: hide an idle cursor.
        if self.mode == WindowMode::Fullscreen && !self.show_sidebar && !self.cursor_hidden && self.last_mouse_move.elapsed() > Duration::from_millis(2500) {
            g.window.set_cursor_visible(false);
            self.cursor_hidden = true;
        }

        self.gfx = Some(g);
        for a in actions {
            match a {
                ui::UiAction::SetMode(m) => self.apply_mode(m),
                ui::UiAction::Scene(s) => self.switch_scene(s),
                ui::UiAction::HideSidebar => self.set_sidebar_visible(false),
            }
        }
        self.update_hittest();

        self.idle = !drawer_open && controls_alpha <= 0.0 && self.toast_t <= 0.0 && self.vis.silent_for > 3.0 && self.scenes.prev.is_none();
        self.stats.update();
        if self.trim_at.is_some_and(|t| Instant::now() >= t) {
            self.trim_at = None;
            sys::trim_working_set();
        }
        self.schedule(el);
    }

    fn schedule(&mut self, el: &ActiveEventLoop) {
        let Some(g) = &self.gfx else { return };
        if self.minimized || self.occluded {
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(200)));
        } else if self.idle {
            let period = Duration::from_secs_f32(1.0 / self.settings.idle_fps.max(1) as f32);
            self.next_frame = self.last_frame + period;
            el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        } else {
            el.set_control_flow(ControlFlow::Wait);
            g.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("BlazeAudio")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(LogicalSize::new(320.0, 200.0))
            .with_window_icon(app_icon())
            .with_taskbar_icon(app_icon())
            .with_no_redirection_bitmap(true);
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                fatal(&format!("Could not create window: {e}"));
                el.exit();
                return;
            }
        };
        let surface = match self.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                fatal(&format!("Could not create DX12 surface: {e}"));
                el.exit();
                return;
            }
        };
        let size = window.inner_size();
        let gpu = match pollster::block_on(Gpu::new(&self.instance, Some(surface), size.width, size.height, self.settings.render_scale)) {
            Ok(g) => g,
            Err(e) => {
                fatal(&e);
                el.exit();
                return;
            }
        };
        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(4096),
        );
        self.gfx = Some(Gfx { window, gpu, egui_ctx, egui_state });
        self.last_frame = Instant::now();
        let start_mode = self.settings.start_mode;
        if start_mode != WindowMode::Windowed {
            self.apply_mode(start_mode);
        }
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }

    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            if self.minimized || self.occluded {
                self.poll_global_hotkeys();
                self.schedule(el);
            } else if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let mut consumed = false;
        if let Some(g) = &mut self.gfx {
            if !matches!(event, WindowEvent::RedrawRequested) {
                let r = g.egui_state.on_window_event(&g.window, &event);
                consumed = r.consumed && self.show_sidebar;
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                self.settings.save();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                self.minimized = size.width == 0 || size.height == 0;
                if let Some(g) = &mut self.gfx {
                    g.gpu.resize(size.width, size.height);
                    g.window.request_redraw();
                }
            }
            WindowEvent::Occluded(o) => {
                #[cfg(debug_assertions)]
                eprintln!("[win] occluded={o}");
                self.occluded = o;
                if !o {
                    if let Some(g) = &self.gfx {
                        g.window.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let wants_kb = self.gfx.as_ref().is_some_and(|g| g.egui_ctx.wants_keyboard_input());
                if event.state == ElementState::Pressed && !event.repeat && !wants_kb && !(consumed && event.physical_key != PhysicalKey::Code(KeyCode::F1)) {
                    if let PhysicalKey::Code(code) = event.physical_key {
                        self.handle_key(el, code);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                self.last_mouse_move = Instant::now();
                if self.cursor_hidden {
                    if let Some(g) = &self.gfx {
                        g.window.set_cursor_visible(true);
                    }
                    self.cursor_hidden = false;
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let over_ui = self.gfx.as_ref().is_some_and(|g| g.egui_ctx.is_pointer_over_area());
                let [x, y, w, h] = self.scene_px;
                let (cx, cy) = (self.cursor.x as f32, self.cursor.y as f32);
                let in_scene = cx >= x && cx < x + w && cy >= y && cy < y + h;
                if in_scene && !over_ui && !consumed {
                    if self.last_click.elapsed() < Duration::from_millis(350) {
                        self.toggle_mode(WindowMode::Fullscreen);
                        self.last_click = Instant::now() - Duration::from_secs(5);
                    } else {
                        self.last_click = Instant::now();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.minimized && !self.occluded {
                    self.frame(el);
                } else {
                    self.schedule(el);
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.settings.save();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let dir = args.get(i + 1).cloned().unwrap_or_else(|| "snapshots".into());
        snapshot::run(std::path::Path::new(&dir));
        return;
    }
    let event_loop = match EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            fatal(&format!("Could not start event loop: {e}"));
            return;
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
