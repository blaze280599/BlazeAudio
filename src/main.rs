#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dock;
mod gpu;
mod media;
mod palette;
mod scenes;
mod settings;
mod snapshot;
mod sys;
mod ui;
mod update;
mod vis;
mod volume;
mod wake;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
use winit::window::{Fullscreen, Icon, ResizeDirection, Window, WindowId};

use audio::{AudioEngine, AudioParams};
use gpu::Gpu;
use media::{MediaWatcher, TrackInfo};
use palette::PRESETS;
use settings::{AutoScene, DockFull, DockSide, Settings, WindowGeom, WindowMode};
use update::Updater;
use vis::VisState;

pub fn make_instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: Default::default(),
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions {
                shader_compiler: wgpu::Dx12Compiler::Fxc,
                ..Default::default()
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

/// Window class, so a second launch can find the running window whatever its title says.
const WINDOW_CLASS: &str = "BlazeAudio";

/// True when another BlazeAudio is already running; that one is brought to the front. Two
/// copies would fight over the settings file. With `wait` (a restart after an update), the
/// old copy gets a few seconds to close first.
fn already_running(wait: bool) -> bool {
    use windows::core::{w, HSTRING};
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let tries = if wait { 50 } else { 1 };
    unsafe {
        for i in 0..tries {
            // Held (never closed) for the life of the process.
            let Ok(mutex) = CreateMutexW(None, false, w!("Local\\BlazeAudio.SingleInstance")) else { return false };
            if GetLastError() != ERROR_ALREADY_EXISTS {
                return false;
            }
            if i + 1 < tries {
                // Our handle would keep the mutex alive after the old copy exits.
                let _ = CloseHandle(mutex);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if let Ok(hwnd) = FindWindowW(&HSTRING::from(WINDOW_CLASS), None) {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        true
    }
}

/// Sets the title-bar and taskbar icons from the exe's icon resource (see build.rs) at the
/// window's DPI; Windows picks the closest size in the icon group.
fn set_icons(w: &Window) {
    use winit::platform::windows::IconExtWindows;
    let icon = |pt: f64| {
        let n = (pt * w.scale_factor()).round() as u32;
        Icon::from_resource(1, Some(PhysicalSize::new(n, n))).ok()
    };
    w.set_window_icon(icon(16.0));
    w.set_taskbar_icon(icon(32.0));
}

/// The window while it's docked beside Spotify.
struct Docked {
    /// Where the window was before docking (outer position, inner size); it goes back there after.
    float: (PhysicalPosition<i32>, PhysicalSize<u32>),
    /// The side it's on now: the chosen one, or the other when the chosen one is off the screen.
    side: DockSide,
    /// Minimized because Spotify was, so it comes back up with Spotify.
    minimized_with: bool,
    /// Hidden while Spotify is maximized (`DockFull::Hide`).
    hidden: bool,
    /// 1 fully shown, 0 faded out.
    opacity: f32,
    fade: Option<Fade>,
}

/// The panel fading out as Spotify minimizes and back in as it restores. Windows animates a
/// minimize only into a taskbar button, which a docked panel lacks.
struct Fade {
    start: Instant,
    /// Fading out rather than in.
    out: bool,
    /// `Docked::opacity` when it started, so a fade reversed halfway carries on from there.
    from: f32,
}

/// Lengths of the fades, close to Windows' own minimize and restore animations.
const FADE_OUT: f32 = 0.15;
const FADE_IN: f32 = 0.2;

/// What the docked window's size and place depend on besides Spotify's window; a change re-docks.
type DockKey = (DockSide, DockFull, u32, u32, WindowMode);

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
    updater: Updater,
    /// The updater's status as of `update_ver`, copied only when it changes.
    update: update::Status,
    update_ver: u64,
    vis: VisState,
    gfx: Option<Gfx>,
    ui: ui::UiState,

    mode: WindowMode,
    windowed_rect: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
    show_sidebar: bool,

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
    toast_t: f32,
    volume: volume::Volume,
    /// Seconds the current scene has been on screen with music playing (for the scene timer).
    scene_clock: f32,

    /// The audio device's name and whether capture runs on it, as of `device_ver`.
    device: (String, bool),
    device_ver: u64,
    /// Opacity of the "nothing playing" hint, eased toward its target.
    hint_alpha: f32,

    stats: sys::ProcStats,
    trim_at: Option<Instant>,
    last_mouse_move: Instant,
    cursor_hidden: bool,
    last_click: Instant,
    ignore_audio: bool,
    /// Scene area in physical pixels (x, y, w, h), i.e. the window minus docked panels.
    scene_px: [f32; 4],
    cursor: PhysicalPosition<f64>,

    dock: dock::Tracker,
    docked: Option<Docked>,
    /// The `DockKey` the window was last docked with; `None` forces a re-dock.
    dock_key: Option<DockKey>,
    dock_status: &'static str,
    /// The panel's edge is being dragged; its width follows the window's until the button is up.
    dock_resizing: bool,
}

impl App {
    fn new() -> Self {
        let settings = Settings::load();
        let audio = AudioEngine::start(audio_params(&settings));
        let media = MediaWatcher::start();
        let updater = Updater::start(settings.updates);
        let pal = PRESETS[settings.preset_palette.min(PRESETS.len() - 1)].1;
        let now = Instant::now();
        Self {
            mode: WindowMode::Windowed,
            instance: make_instance(),
            audio,
            media,
            updater,
            update: update::Status::Idle,
            update_ver: 0,
            vis: VisState::new(pal),
            gfx: None,
            ui: ui::UiState::new(),
            windowed_rect: None,
            // A docked panel starts as just the scene; its show-sidebar button brings the sidebar in.
            show_sidebar: settings.start_mode == WindowMode::Windowed && settings.dock == DockSide::Off,
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
            toast_t: 0.0,
            volume: volume::Volume::new(),
            scene_clock: 0.0,
            device: (String::new(), false),
            device_ver: 0,
            hint_alpha: 0.0,
            stats: sys::ProcStats::new(),
            trim_at: Some(now + Duration::from_secs(4)),
            last_mouse_move: now,
            cursor_hidden: false,
            last_click: now - Duration::from_secs(5),
            ignore_audio: std::env::var_os("BLAZE_IGNORE_AUDIO").is_some(),
            scene_px: [0.0, 0.0, f32::MAX, f32::MAX],
            cursor: PhysicalPosition::new(0.0, 0.0),
            dock: dock::Tracker::new(),
            docked: None,
            dock_key: None,
            dock_status: "",
            dock_resizing: false,
            settings,
        }
    }

    fn set_sidebar_visible(&mut self, v: bool) {
        if self.show_sidebar && !v {
            self.ui.drawer = None;
            self.settings.save();
            self.trim_at = Some(Instant::now() + Duration::from_secs(2));
        }
        self.show_sidebar = v;
        if v {
            if let Some(g) = &self.gfx {
                g.window.focus_window();
                g.window.set_cursor_visible(true);
                self.cursor_hidden = false;
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
                if let Some((pos, size)) = self.windowed_rect {
                    w.set_outer_position(pos);
                    let _ = w.request_inner_size(size);
                }
            }
            WindowMode::Fullscreen => w.set_fullscreen(Some(Fullscreen::Borderless(None))),
        }
        self.mode = mode;
        // The sidebar is part of the windowed app; fullscreen is for the visuals.
        self.set_sidebar_visible(mode == WindowMode::Windowed);
        #[cfg(debug_assertions)]
        eprintln!("[win] mode={mode:?} size={:?} pos={:?}", w.inner_size(), w.outer_position());
        w.set_cursor_visible(true);
        self.cursor_hidden = false;
        self.trim_at = Some(Instant::now() + Duration::from_secs(3));
    }

    /// Records where the windowed-mode window is, so the next launch opens in the same place.
    fn remember_window(&mut self) {
        let Some(g) = &self.gfx else { return };
        let w = &g.window;
        let prev = self.settings.window;
        let rect = match self.mode {
            // Docked, it's wherever Spotify is; the next launch floats where it was before docking.
            _ if self.docked.is_some() => self.docked.as_ref().map(|d| d.float),
            WindowMode::Windowed if w.is_minimized() == Some(true) => return,
            // A maximized window's rect is the whole monitor; keep the restored rect from
            // before and only note that it was maximized.
            WindowMode::Windowed if w.is_maximized() => {
                let cur = w.outer_position().ok().map(|p| WindowGeom { x: p.x, y: p.y, w: w.inner_size().width, h: w.inner_size().height, maximized: true });
                self.settings.window = prev.map(|p| WindowGeom { maximized: true, ..p }).or(cur);
                return;
            }
            WindowMode::Windowed => w.outer_position().ok().map(|p| (p, w.inner_size())),
            _ => self.windowed_rect,
        };
        if let Some((pos, size)) = rect {
            let maximized = (self.mode != WindowMode::Windowed || self.docked.is_some()) && prev.is_some_and(|p| p.maximized);
            self.settings.window = Some(WindowGeom { x: pos.x, y: pos.y, w: size.width, h: size.height, maximized });
        }
    }

    fn dock_key(&self) -> DockKey {
        let s = &self.settings;
        let panels = ui::panels_width(self.show_sidebar, s.sidebar_expanded, self.ui.drawer.is_some());
        (s.dock, s.dock_full, s.dock_width.to_bits(), panels.to_bits(), self.mode)
    }

    /// Puts the window where docking says: beside Spotify, minimized or hidden along with it, or
    /// back to a normal window. Runs on Spotify's window events and when a dock input changes.
    fn dock_sync(&mut self) {
        dock::clear_pending();
        self.dock_key = Some(self.dock_key());
        let Some(w) = self.gfx.as_ref().map(|g| g.window.clone()) else { return };
        let Some(ours) = dock::hwnd(&w) else { return };
        if self.settings.dock == DockSide::Off {
            self.dock.release();
            self.undock();
            return;
        }
        // Fullscreen covers everything; docking picks up again on the way back to windowed.
        if self.mode != WindowMode::Windowed {
            return;
        }
        let Some((target, t)) = self.dock.target().and_then(|h| Some((h, dock::info(h)?))) else {
            self.undock();
            self.dock_status = "Waiting for Spotify. The panel docks once its window is open.";
            return;
        };
        // The panel goes down with Spotify and comes back up with it, even if it was minimized on
        // its own first. It hides rather than minimizes: with no taskbar button, a minimized
        // window would be parked as a stub title bar in a corner of the desktop.
        if t.minimized {
            if let Some(d) = self.docked.as_mut().filter(|d| !d.hidden && !d.minimized_with) {
                d.minimized_with = true;
                if w.is_minimized() != Some(true) {
                    if dock::animations_on() {
                        d.fade = Some(Fade { start: Instant::now(), out: true, from: d.opacity });
                        w.request_redraw();
                    } else {
                        dock::hide(ours);
                    }
                }
            }
            return;
        }
        if t.fills {
            self.dock_status = "Spotify is maximized, so there's no room beside it.";
            match self.settings.dock_full {
                DockFull::Float => {
                    self.undock();
                    // Its pre-dock spot is usually under the maximized Spotify, where it would look
                    // gone; keep it above Spotify whenever Spotify comes to the front.
                    if t.foreground {
                        dock::raise(ours);
                    }
                }
                DockFull::Hide => {
                    if self.docked.is_none() {
                        self.begin_dock(&w);
                    }
                    if let Some(d) = self.docked.as_mut().filter(|d| !d.hidden) {
                        dock::hide(ours);
                        (d.hidden, d.minimized_with) = (true, false);
                    }
                }
            }
            return;
        }
        // Minimized by the user rather than with Spotify: it stays down until they bring it back.
        if self.docked.as_ref().is_some_and(|d| !d.minimized_with && !d.hidden) && w.is_minimized() == Some(true) {
            return;
        }
        if self.docked.is_none() {
            self.begin_dock(&w);
        }
        let panels = ui::panels_width(self.show_sidebar, self.settings.sidebar_expanded, self.ui.drawer.is_some());
        let width = ((self.settings.dock_width + panels) * w.scale_factor() as f32).round() as i32;
        let Some(d) = &mut self.docked else { return };
        // Switch sides when the chosen one would be mostly off the screen and the other has room
        // (Spotify snapped to that half of the screen, say).
        let r = t.rect;
        let room = |side: DockSide, part: i32| match side {
            DockSide::Left => r.left - width / part >= t.work.left,
            _ => r.right + width / part <= t.work.right,
        };
        let other = if self.settings.dock == DockSide::Left { DockSide::Right } else { DockSide::Left };
        d.side = if !room(self.settings.dock, 2) && room(other, 1) { other } else { self.settings.dock };
        let x = if d.side == DockSide::Left { r.left - width } else { r.right };
        dock::place(ours, x, r.top, width, r.bottom - r.top);
        if d.minimized_with || d.hidden {
            // Back with Spotify: fade in (from wherever a fade out had got to).
            if d.minimized_with && dock::animations_on() {
                if d.fade.is_none() {
                    d.opacity = 0.0;
                }
                d.fade = Some(Fade { start: Instant::now(), out: false, from: d.opacity });
            } else {
                (d.opacity, d.fade) = (1.0, None);
            }
            dock::set_alpha(ours, d.fade.is_some().then_some(d.opacity));
            dock::show(ours);
            // Showing a window again can bring its taskbar button back.
            w.set_skip_taskbar(true);
            (d.minimized_with, d.hidden) = (false, false);
            w.request_redraw();
        }
        dock::stack_above(target, ours);
        self.dock_status = "Docked. Drag the panel's outer edge to resize it.";
    }

    /// Moves a running fade on to where it should be by now, and ends it: hidden after a fade
    /// out, opaque (no longer layered) after a fade in. Driven by frames and, in case the window
    /// stops getting frames, by a short timer (see `schedule`).
    fn dock_fade(&mut self) {
        let Some(ours) = self.gfx.as_ref().and_then(|g| dock::hwnd(&g.window)) else { return };
        let Some(d) = &mut self.docked else { return };
        let Some(f) = &d.fade else { return };
        let p = (f.start.elapsed().as_secs_f32() / if f.out { FADE_OUT } else { FADE_IN }).min(1.0);
        // Easing in on the way out and out on the way in, like Windows' own animations.
        let (to, e) = if f.out { (0.0, p * p) } else { (1.0, 1.0 - (1.0 - p) * (1.0 - p)) };
        d.opacity = f.from + (to - f.from) * e;
        dock::set_alpha(ours, Some(d.opacity));
        if p >= 1.0 {
            if f.out {
                dock::hide(ours);
            } else {
                dock::set_alpha(ours, None);
            }
            d.fade = None;
        }
    }

    /// Starts docking: remembers where the window floats, drops its frame and its taskbar button
    /// (the panel is part of Spotify now).
    fn begin_dock(&mut self, w: &Window) {
        if w.is_maximized() {
            w.set_maximized(false);
        }
        let pos = w.outer_position().unwrap_or_default();
        self.docked = Some(Docked {
            float: (pos, w.inner_size()),
            side: self.settings.dock,
            minimized_with: false,
            hidden: false,
            opacity: 1.0,
            fade: None,
        });
        w.set_decorations(false);
        w.set_skip_taskbar(true);
        if let Some(ours) = dock::hwnd(w) {
            dock::round_corners(ours, true);
        }
    }

    /// Back to a normal window where it was before docking.
    fn undock(&mut self) {
        let Some(d) = self.docked.take() else { return };
        let Some(g) = &self.gfx else { return };
        if let Some(ours) = dock::hwnd(&g.window) {
            dock::set_alpha(ours, None);
            if d.hidden || d.minimized_with {
                dock::show(ours);
            }
            dock::round_corners(ours, false);
        }
        g.window.set_decorations(true);
        g.window.set_skip_taskbar(false);
        if self.mode == WindowMode::Windowed {
            g.window.set_outer_position(d.float.0);
            let _ = g.window.request_inner_size(d.float.1);
        } else {
            self.windowed_rect = Some(d.float);
        }
        g.window.request_redraw();
    }

    fn toggle_mode(&mut self, m: WindowMode) {
        let target = if self.mode == m { WindowMode::Windowed } else { m };
        self.apply_mode(target);
    }

    fn sync_media(&mut self) {
        let (tv, av) = self.media.versions();
        if tv != self.track_ver || av != self.art_ver {
            let new_track = tv != self.track_ver;
            let new_art = av != self.art_ver;
            self.track_ver = tv;
            self.art_ver = av;
            let had_track = !self.track.title.is_empty();
            self.track = self.media.snapshot();
            if new_track {
                let t = &self.track;
                if !t.title.is_empty() {
                    self.toast_t = 6.0;
                    // Not for the first track seen, so launching the app doesn't skip a scene.
                    if had_track && self.settings.auto_scene == AutoScene::Track {
                        self.settings.visual = self.settings.visual.next();
                    }
                }
                if let Some(g) = &self.gfx {
                    g.window.set_title(&match (t.title.is_empty(), t.artist.is_empty()) {
                        (true, _) => "BlazeAudio".to_owned(),
                        (false, true) => format!("{} · BlazeAudio", t.title),
                        (false, false) => format!("{} – {} · BlazeAudio", t.title, t.artist),
                    });
                }
            }
            if let Some(g) = &mut self.gfx {
                if new_art {
                    self.ui.set_art(&g.egui_ctx, self.track.art.as_deref().map(|v| v.as_slice()));
                }
                g.gpu.set_track(&self.track);
            }
        }
        self.track.playback = self.media.playback();
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
        self.dock_fade();
        self.sync_media();
        g = self.gfx.take().unwrap();

        if self.audio.device_version() != self.device_ver {
            self.device_ver = self.audio.device_version();
            self.device = self.audio.device();
        }
        if self.updater.version() != self.update_ver {
            self.update_ver = self.updater.version();
            self.update = self.updater.status();
        }

        self.toast_t = (self.toast_t - dt).max(0.0);

        // Audio -> visual state.
        // BLAZE_IGNORE_AUDIO=1 renders as if nothing is playing (for measuring the idle path).
        let frame = if self.ignore_audio { audio::AudioFrame::default() } else { self.audio.snapshot() };
        self.vis.update(&frame, dt, now);

        // UI runs first: the docked sidebar decides how much of the window the scene gets.
        // egui only runs when something is on screen, so idle frames cost no UI work.
        let toast_alpha = (self.toast_t / 0.6).min(1.0).min((6.0 - self.toast_t) / 0.4).clamp(0.0, 1.0);
        // Resting on the volume bar or timeline keeps the controls up.
        let still = if self.ui.controls_hot { 0.0 } else { self.last_mouse_move.elapsed().as_secs_f32() };
        let controls_alpha = (1.0 - (still - 2.0) / 0.5).clamp(0.0, 1.0);
        let volume_alpha = if self.settings.show_volume { controls_alpha } else { 0.0 };
        let has_timeline = self.settings.show_timeline && !self.track.title.is_empty() && self.track.playback.duration > 0.0;
        let timeline_alpha = if has_timeline { controls_alpha } else { 0.0 };
        // A hint over a silent scene with no track. Without the sidebar it shows only for the
        // first seconds after launch or while the mouse moves, so idle frames stay free of UI.
        let empty = self.settings.show_track_toast && self.track.title.is_empty() && (self.vis.silent_for > 3.0 || !self.device.1);
        let hint_target = if empty && (self.show_sidebar || controls_alpha > 0.0 || time < 12.0) { 1.0 } else { 0.0 };
        self.hint_alpha += (hint_target - self.hint_alpha) * (1.0 - (-dt / 0.25).exp());
        if (hint_target - self.hint_alpha).abs() < 0.01 {
            self.hint_alpha = hint_target;
        }
        let need_ui = self.show_sidebar
            || self.hint_alpha > 0.0
            || (self.toast_t > 0.0 && self.settings.show_track_toast)
            || self.settings.show_stats
            || controls_alpha > 0.0
            || volume_alpha > 0.0
            || timeline_alpha > 0.0;
        let drawer_was_open = self.ui.drawer.is_some();
        let mut actions = Vec::new();
        let mut egui_out = None;
        let (sw, sh) = (g.gpu.config.width as f32, g.gpu.config.height as f32);
        self.scene_px = [0.0, 0.0, sw, sh];
        if need_ui {
            let raw = g.egui_state.take_egui_input(&g.window);
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
                        mode: self.mode,
                        stats: &self.stats,
                        fps: self.fps,
                        frame_ms: self.frame_ms,
                        idle: self.idle,
                        device: &self.device.0,
                        capturing: self.device.1,
                        hint_alpha: self.hint_alpha,
                        adapter: &g.gpu.adapter_name,
                        show_sidebar: self.show_sidebar,
                        toast_alpha,
                        controls_alpha,
                        volume: &self.volume,
                        volume_alpha,
                        timeline_alpha,
                        update: &self.update,
                        dock_edge: self.docked.as_ref().filter(|d| !d.hidden).map(|d| d.side),
                        dock_status: self.dock_status,
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
        // The scene timer only runs while music plays, so a paused track holds the scene.
        if self.vis.silent_for < 1.0 {
            self.scene_clock += dt;
        }
        if self.settings.auto_scene == AutoScene::Timer && self.scene_clock >= self.settings.auto_scene_minutes * 60.0 {
            self.settings.visual = self.settings.visual.next();
        }
        if g.gpu.scene != self.settings.visual {
            g.gpu.scene = self.settings.visual;
            self.vis.scene_time = 0.0;
            self.scene_clock = 0.0;
        }
        let globals = self.vis.globals(&self.settings, g.gpu.scene_size(), time);
        g.gpu.upload(&globals, &self.vis, &self.settings.scenes);

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
        g.gpu.encode(&mut enc, &view);

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
                ui::UiAction::HideSidebar => self.set_sidebar_visible(false),
                ui::UiAction::ShowSidebar => self.set_sidebar_visible(true),
                ui::UiAction::Dock => {
                    self.settings.dock = if self.settings.dock_last == DockSide::Off { DockSide::Right } else { self.settings.dock_last };
                    self.settings.save();
                }
                ui::UiAction::Undock => {
                    if self.settings.dock != DockSide::Off {
                        self.settings.dock_last = self.settings.dock;
                    }
                    self.settings.dock = DockSide::Off;
                    self.settings.save();
                }
                ui::UiAction::Media(cmd) => self.media.send(cmd),
                ui::UiAction::Update(cmd) => self.updater.send(cmd),
                ui::UiAction::Restart => {
                    self.remember_window();
                    self.settings.save();
                    if self.updater.relaunch() {
                        el.exit();
                    }
                }
                ui::UiAction::DockResize => {
                    if let (Some(d), Some(g)) = (&self.docked, &self.gfx) {
                        let dir = if d.side == DockSide::Left { ResizeDirection::West } else { ResizeDirection::East };
                        self.dock_resizing = g.window.drag_resize_window(dir).is_ok();
                    }
                }
            }
        }

        // Re-dock when a setting, the sidebar or the mode changed the panel, or when an edge drag
        // ended. Without a Spotify window, look for one (the tracker limits how often).
        if self.dock_resizing && !dock::left_button_down() {
            self.dock_resizing = false;
            self.dock_key = None;
        }
        if !self.dock_resizing
            && (self.dock_key != Some(self.dock_key())
                || (self.settings.dock != DockSide::Off && self.mode == WindowMode::Windowed && !self.dock.following())
                || self.dock.lost())
        {
            self.dock_sync();
        }

        self.idle = !drawer_open && controls_alpha <= 0.0 && volume_alpha <= 0.0 && timeline_alpha <= 0.0 && self.toast_t <= 0.0
            && self.hint_alpha == hint_target
            && self.vis.silent_for > 3.0;
        self.stats.update();
        if self.trim_at.is_some_and(|t| Instant::now() >= t) {
            self.trim_at = None;
            sys::trim_working_set();
        }
        self.schedule(el);
    }

    fn schedule(&mut self, el: &ActiveEventLoop) {
        let Some(g) = &self.gfx else { return };
        // Hidden along with Spotify counts as minimized, once the fade out has finished.
        let fading = self.docked.as_ref().is_some_and(|d| d.fade.is_some());
        let docked_away = !fading && self.docked.as_ref().is_some_and(|d| d.hidden || d.minimized_with);
        if fading {
            // Frames move the fade; the timer moves it too if the window stops getting frames
            // (covered by another window, say), so it can't get stuck halfway.
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(8)));
            g.window.request_redraw();
        } else if docked_away {
            // Nothing to draw, but check now and then that Spotify is still there: if it was killed
            // without a destroy event, nothing else would bring the panel back.
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(2)));
        } else if self.minimized || self.occluded {
            // Nothing to draw; restoring the window (Resized / Occluded) requests the next frame.
            el.set_control_flow(ControlFlow::Wait);
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

impl ApplicationHandler<dock::Wake> for App {
    /// Spotify's window moved, resized, minimized, came to the front or went away.
    fn user_event(&mut self, _el: &ActiveEventLoop, _: dock::Wake) {
        if !self.dock_resizing {
            self.dock_sync();
        }
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("BlazeAudio")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(LogicalSize::new(320.0, 200.0))
            .with_class_name(WINDOW_CLASS);
        // Reopen where it was, as long as its title bar is still on a connected monitor.
        if let Some(gm) = self.settings.window {
            let (tx, ty) = (gm.x + gm.w as i32 / 2, gm.y + 16);
            let visible = el.available_monitors().any(|m| {
                let (p, s) = (m.position(), m.size());
                tx >= p.x && tx < p.x + s.width as i32 && ty >= p.y && ty < p.y + s.height as i32
            });
            if visible && gm.w >= 320 && gm.h >= 200 {
                attrs = attrs
                    .with_position(PhysicalPosition::new(gm.x, gm.y))
                    .with_inner_size(PhysicalSize::new(gm.w, gm.h))
                    .with_maximized(gm.maximized);
            }
        }
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                fatal(&format!("Could not create window: {e}"));
                el.exit();
                return;
            }
        };
        set_icons(&window);
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
            if self.docked.as_ref().is_some_and(|d| d.fade.is_some()) {
                self.dock_fade();
                self.schedule(el);
            } else if self.docked.as_ref().is_some_and(|d| d.hidden || d.minimized_with) {
                if self.dock.lost() {
                    self.dock_sync();
                }
                self.schedule(el);
            } else if self.minimized || self.occluded {
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
                self.remember_window();
                self.settings.save();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                let was_minimized = self.minimized;
                self.minimized = size.width == 0 || size.height == 0;
                // Restored while docked: Spotify may have moved meanwhile, so re-dock. Brought back
                // by hand (Win+Shift+M, say) while Spotify is still minimized: bring Spotify back too,
                // or the re-dock would send the panel straight back down.
                if was_minimized && !self.minimized && self.docked.is_some() {
                    self.dock_key = None;
                    if let Some(t) = self.dock.current().filter(|t| dock::info(*t).is_some_and(|i| i.minimized)) {
                        dock::show(t);
                    }
                }
                // Dragging the docked panel's edge: the scene takes the new width.
                if self.dock_resizing && !self.minimized {
                    if let Some(g) = &self.gfx {
                        let panels = ui::panels_width(self.show_sidebar, self.settings.sidebar_expanded, self.ui.drawer.is_some());
                        self.settings.dock_width = (size.width as f32 / g.window.scale_factor() as f32 - panels).round().clamp(320.0, 1600.0);
                        self.dock_key = Some(self.dock_key());
                    }
                }
                if let Some(g) = &mut self.gfx {
                    g.gpu.resize(size.width, size.height);
                    g.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(g) = &self.gfx {
                    set_icons(&g.window);
                }
                // Moved onto a monitor with another scale: the panel's width in pixels changes.
                self.dock_key = None;
            }
            // Clicking the docked panel brings Spotify up with it.
            WindowEvent::Focused(true) => {
                if let (Some(_), Some(target), Some(g)) = (&self.docked, self.dock.current(), &self.gfx) {
                    if let Some(ours) = dock::hwnd(&g.window) {
                        dock::raise_target_under(target, ours);
                    }
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
    if already_running(args.iter().any(|a| a == "--relaunch")) {
        return;
    }
    let event_loop = match EventLoop::<dock::Wake>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            fatal(&format!("Could not start event loop: {e}"));
            return;
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    dock::init(event_loop.create_proxy());
    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
