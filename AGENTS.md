# AGENTS.md

A guide for AI coding agents (and humans who want the full picture) working on BlazeAudio. It covers what the app is, how it's built, the rules that keep it working, and step-by-step recipes for the changes you're most likely to be asked for. `README.md` is the short, user-facing overview. This file is the reference.

Read the whole of this file before your first change in a session. When you finish a change, check the "Keeping this file current" section at the end.

---

## 1. What the app is

BlazeAudio is a single-binary Windows music visualizer written in Rust.

- It **captures whatever the PC is playing** through WASAPI loopback (no driver, no virtual cable).
- It **reads now-playing info and album art** from the Windows media session API (GSMTC), so Spotify or a browser shows title, artist and cover. Play, pause, skip and seek go back through the same API.
- It **renders audio-reactive HDR scenes** with wgpu on DirectX 12, followed by bloom, ACES tonemapping and film effects.
- It has **two window modes**: windowed with a docked egui sidebar, and borderless fullscreen.
- It **controls the system output volume** through Core Audio.
- It **updates itself** from this repo's GitHub releases. It's a portable exe, so the update replaces the exe in place.

The current scenes are **Faulty Terminal** (CRT glyph spectrum), **Gradient ASCII** (a light seen through a glyph grid) **Phosphor Scope** (a vector oscilloscope with afterglow) and **Crystalized Ball** (an electric glass sphere full of instanced dust particles). More scenes are planned; the scene framework in `src/scenes/` exists for that reason.

### Hard constraints

These are product decisions. Don't change them without the user asking.

| Constraint | Consequence |
|---|---|
| Windows only | The `windows` crate, WASAPI, GSMTC and DX12 are used directly. No cross-platform abstractions or `cfg` gates for other OSes. |
| DX12 only, FXC shader compiler | `make_instance()` in `main.rs` selects `Backends::DX12` and `Dx12Compiler::Fxc`. Shaders must compile under FXC (see Pitfalls §9). |
| Portable, one self-contained `.exe` | No installer and no code signing. Static CRT (`.cargo/config.toml`), no runtime asset files. Shaders, fonts and icons are embedded (`include_str!`, `epaint_default_fonts`, painted icons). Don't add files the exe must load at runtime. |
| Very low idle cost | About 0.1% CPU idle and 0.6% playing, about 20 MB working set. Don't add per-frame allocations in hot paths, polling threads, or work that runs while silent or hidden. |
| Network only for update checks | The one network use is `update.rs` asking GitHub for the latest release and downloading it, which the Updates setting can turn off. Metadata comes from the OS, not web APIs. Don't add other network calls, telemetry or HTTP crates (`update.rs` uses WinRT `HttpClient` from the `windows` crate). |
| Single instance | A named mutex plus `FindWindowW` on the window class `BlazeAudio`. A second launch brings the first window to the front and exits. `--relaunch` (used by Restart now after an update) instead waits up to 5 s for the old copy to close. |

---

## 2. Build, run, test

The toolchain is stable Rust on MSVC, plus the Visual Studio C++ build tools. The shell is usually Git Bash or PowerShell on Windows.

```bash
cargo build                          # dev build; dependencies are built at opt-level 2, so dev runs are representative
```

```bash
cargo build --release                # fat LTO, codegen-units=1, panic=abort, stripped; slow to link
```

```bash
cargo test                           # unit tests (currently settings migration)
```

```bash
cargo clippy --all-targets           # the tree has ~40 pre-existing warnings; add none in code you touch
```

```bash
cargo run -- --snapshot snapshots    # headless self-test: PNGs + report.txt, no window
```

- A release build has no console (`windows_subsystem = "windows"`). Dev builds keep one, and `#[cfg(debug_assertions)] eprintln!` diagnostics go there with prefixes like `[win]` and `[audio]`.
- `panic = "abort"` in release: any panic, including `.unwrap()` on a poisoned mutex, kills the process without unwinding. Avoid new panicking paths on the main or worker threads.
- Running the real app reads and writes `%APPDATA%\BlazeAudio\settings.json`. For test runs, point it at a scratch folder so you don't change the user's settings: set `APPDATA` for that process only.
- Only one copy runs at a time. If the user already has BlazeAudio open, launching yours just focuses theirs and exits immediately. Check with `tasklist | grep -i blazeaudio`.
- Close it gracefully (`taskkill //IM blazeaudio.exe` from Git Bash, without `/F`) so it saves settings the way a real close does.

### Environment variables

| Variable | Effect |
|---|---|
| `BLAZE_IGNORE_AUDIO=1` | Render as if silent, to measure the idle path. |
| `BLAZE_TRACE=<file.csv>` | Dump about 10 s of the live onset/threshold curve from the analyzer. |
| `BLAZE_UPDATE_URL=<url>` | Dev builds only: ask this URL instead of GitHub for the latest release, and check on launch. `scripts/update-test.sh` uses it to test an update end to end. |

### The snapshot self-test (`snapshot.rs`)

`--snapshot <dir>` runs without a window:

1. It synthesizes a 124 BPM track (kick, off-beat hats, snare, saw bass, pad).
2. It feeds 8 s of it through the real analyzer (`audio::Offline`) and checks beat count and phase.
3. It renders each scene offscreen at 1280×720 with a fixed 1/60 s step: `terminal.png`, `terminal_beat.png`, `ascii_{horizon,pinch,rise,eclipse,intro}.png`, `scope_{yt,polar}.png` and `ball{,_beat,_fall,_intro}.png`.
4. It writes `report.txt` (adapter, init time, ms/frame per scene, beats detected vs expected, BPM, beat phases) and `onset_trace.csv`.

**The run is deterministic.** For a refactor or any change that shouldn't alter the visuals, build the old commit, snapshot into `before/`, build yours, snapshot into `after/`, and byte-compare the PNGs (`cmp`). They must be identical. For changes that are meant to alter the visuals, look at the PNGs; the Read tool can open images. Also check `report.txt`: beats should be about 21 over about 10.3 s with a BPM near 124, and phases should cluster near 0.0x.

Python on this machine may not have PIL, so compare files with `cmp` or a hash rather than an image library.

---

## 3. Architecture

### 3.1 Threads

| Thread | Owner | Produces | Consumed via |
|---|---|---|---|
| `audio-capture` | `audio.rs` `capture_loop` | `AudioFrame` (bands, waveform, beat count/strength, hat, energy, BPM, level) | `AudioEngine::snapshot()` clones it out of a `Mutex` |
| `media-session` | `media.rs` `watch_loop` | `TrackInfo` (title, artist, album, art RGBA, palette, `Playback`) plus `track_version` / `art_version` counters | `MediaWatcher::snapshot()`, `versions()`, `playback()`; commands in through an `mpsc` channel |
| `volume` | `volume.rs` | Level and mute as atomics | `Volume::level()` / `muted()`; changes go out through a channel |
| `update` | `update.rs` `worker` | `update::Status` plus a `version` counter | `Updater::version()` / `status()`; `UpdateCmd::{Check, Install}` in through a channel |
| main (winit) | `main.rs` `App` | Everything else | — |

Rules:

- **The main thread never blocks on a worker.** Workers publish snapshots behind short-lived locks or atomics, and the main thread copies them. Keep it that way: no `join`, no `recv()` without a timeout, no COM calls on the main thread.
- Each worker owns its OS resources. COM and WinRT are initialized on the worker (`CoInitializeEx` / `RoInitialize` multithreaded) and never cross threads.
- Workers survive failures by retrying with sleeps. A missing device or session leads to a back-off and a retry, never a panic. The audio loop rebuilds its stream on stream errors (ignoring `Xrun`, which loopback reports routinely) and when the default output device changes, checked every 2 s.
- Analysis runs **inside the cpal callback** at a fixed 256-sample hop. It depends on sample time, not frame rate, and when the PC is silent WASAPI stops delivering packets, so analysis costs nothing.

### 3.2 The frame loop (`App::frame` in `main.rs`)

```
sync_media()                                             ← track/art changes, palette target, auto-switch on track
vis.update(audio.snapshot(), dt)                         ← smoothing, kick/hat envelopes, flow clock, palette crossfade
need_ui? → egui_ctx.run(ui.draw(UiCtx{..}))              ← returns Vec<UiAction> + the scene rect
gpu.set_viewport(scene rect) / set_render_scale
scene timer / auto-switch → settings.visual → gpu.scene
globals = vis.globals(settings, gpu.scene_size(), time)
gpu.upload(&globals, &vis, &settings.scenes)              ← Globals + audio buffer + active scene's Params
surface.get_current_texture()                            ← Outdated/Lost → resize and skip; OOM → exit
gpu.encode(): scene → bloom (6 levels) → tonemap into viewport
gpu.encode_egui()                                        ← UI on top
submit, present
drawer open → audio.set_params(); drawer just closed → settings.save()
apply UiActions (SetMode, HideSidebar, Media, Update, Restart)
idle = nothing on screen needs updates && silent > 3 s
schedule(): ControlFlow::Wait + request_redraw | WaitUntil(idle period) | Wait when minimized/occluded
```

Key facts:

- **egui only runs when something is visible**: the sidebar, toast, stats, on-scene controls, the volume bar or the timeline. The `need_ui` expression lists them. Anything new that draws over the scene must be added there and to `idle`, or it will freeze or vanish.
- **The UI runs before rendering** because the docked sidebar decides the scene viewport. Scene targets are sized to the viewport times `render_scale`. When that size changes, `Gpu::rebuild_targets` rebuilds the bloom chain and calls `Scene::resize` on every scene.
- **Frame pacing**: when active, vsync (`AutoVsync`) paces frames through `request_redraw`. When idle, `WaitUntil(last_frame + 1/idle_fps)`. When minimized or occluded, it waits with no timer; `Resized` / `Occluded(false)` request the next frame.
- **Settings save points**: the drawer closing, the sidebar being hidden, Esc closing the drawer, `CloseRequested` (which also calls `remember_window()`), and `exiting`. Saving is atomic: write `.json.tmp`, then rename.
- `trim_at` schedules `sys::trim_working_set()` a few seconds after startup and mode changes. It's a cosmetic memory-footprint measure.

### 3.3 Window modes and input (`main.rs`)

- **Windowed** shows the sidebar and decorations. **Fullscreen** is `Fullscreen::Borderless`, hides the sidebar, and hides the cursor after 2.5 s without movement.
- The swapchain is an ordinary opaque HWND flip-model swapchain (wgpu's DX12 default).
- `handle_key` ignores Ctrl and Alt combos (except Alt+Enter).
- Keys: `handle_key` gets presses that aren't repeats, except the arrow keys, which repeat for volume and seek. It skips keys when egui wants the keyboard (a text field is focused) or consumed the event (except F1).
- Double-click on the scene, outside any UI area, toggles fullscreen (`MouseInput` plus `scene_px`).
- Window geometry: `remember_window()` stores the restored windowed rect and the maximized flag in `settings.window`. On startup it's reused only if the title bar lands on a connected monitor.

### 3.4 Rendering (`gpu.rs`, `scenes/`, `shaders/`)

Pipeline per frame: **scene → Rgba16Float scene target → bloom downsample ×6 (13-tap, Karis on the first level, soft-knee threshold) → tent upsample (additive) → tonemap (fitted ACES, chromatic aberration on kicks, vignette, grain and dither in display space) → swapchain viewport → egui.**

Ownership:

- **`gpu.rs`** owns the device, queue and surface config, `Globals` and audio buffers, the `Params` buffer, the shared bind group `g0`, `Layouts`, the bloom and tonemap pipelines, the scene target and bloom textures (`Targets`), the egui renderer, and `Vec<Box<dyn Scene>>` in `SceneId::ALL` order.
- **Each scene** owns its pipelines, extra textures (glyph atlas, afterglow ping-pong) and its `Params` layout.

Bindings (identical layout for every pipeline):

| Slot | WGSL | Contents | Defined in |
|---|---|---|---|
| g0 b0 | `G: Globals` (uniform) | res, inv_res, time, treble, hat, kick, beat_count, flow, intensity, bloom params, exposure, chromatic, grain, vignette, energy, beat_age, scene_time, pal[4] | `gpu::Globals` ⇄ `common.wgsl`, filled by `VisState::globals` |
| g0 b1 | `A: AudioData` (storage, read) | `bands[64]`, `wave[2048]` | `VisState::audio_buffer` ⇄ `common.wgsl` |
| g0 b2 | `samp_clamp` | Linear, clamp, linear mips | `gpu.rs` |
| g0 b3 | `P: Params` (uniform, 256 bytes) | The **active** scene's own values | each scene's `.rs` struct ⇄ its `.wgsl` |
| g1 | `texture_2d<f32>` ×1 (or ×2 for tonemap) | Whatever the pass reads | per pipeline |

- `common.wgsl` is **prepended** to every module by `GpuCtx::module`. Everything in it is in scope in every shader: `G`, `A`, `samp_clamp`, `vs_full` (full-screen triangle with `uv` 0..1, y down), `hash11`, `hash21`, `noise2`, `pal(i)`, `band(x)` (continuous band lookup), `luma`, `crt_curve`, `crt_finish`, `PI`, `TAU` and `WAVE_LEN`. Don't redeclare these names in a scene shader.
- Shaders whose pipelines don't use a binding simply don't declare it. Unused bindings in the layout are fine.
- Every scene is built at startup on its own scoped thread, because FXC is slow and single-threaded per shader. Post pipelines build on another. Startup is about 0.5 s including shader compilation. A scene's `new()` must therefore be `Send`-safe and shouldn't do anything slow beyond compiling its own pipelines.
- `Gpu::upload` calls `params()` on the active scene only. `Gpu::encode` passes `fresh = true` on the first frame after a switch.
- Palette colours are **linear RGB** (`palette::Palette = [[f32; 3]; 4]`: primary, secondary, accent, background tint). `vis.pal_cur` crossfades toward `pal_target` over about 0.9 s.
- `capture()` and `step_offscreen()` exist for the self-test. They call `encode()` too, so each counts as a frame for stateful scenes such as the scope's ping-pong.

### 3.5 Audio analysis (`audio.rs`)

Each hop does the following:

1. Blackman-Harris window and a 2048-point `realfft`.
2. 64 log-spaced bands from 30 Hz to 16 kHz, with +3 dB/oct pink compensation and fast-attack / slow-release AGC.
3. Per-band attack/decay smoothing (`AudioParams` from settings, pushed live while the drawer is open).
4. Beats from SuperFlux-style spectral flux (lag 4, max-filtered, kick-weighted) against an adaptive mean + kσ threshold.
5. Tempo from autocorrelation of the onset envelope over 5.5 s, with a log-normal prior at 120 BPM. Once locked, a refractory window stops off-beat hats from registering as beats.

`AudioFrame.beat_count` increments per beat. `VisState` turns that into the `kick` envelope and `beat_age`. `audio::Offline` runs the same analyzer synchronously for the self-test.

### 3.6 Media session (`media.rs`)

- `pick_session` prefers Spotify when several apps publish sessions.
- The watcher polls every 1 s, faster (200–700 ms) briefly after a command or a track change. Album art is fetched with retries for 4 s after a change, because covers often arrive late. The encoded bytes are hashed to skip re-decoding.
- The art is decoded to 256×256 RGBA, and `palette::extract` runs k-means in Oklab, scored for vividness and hue diversity, then pushed toward bright emissive colours.
- Commands are queued, and a run of seeks collapses to the last one. `seek_hold` keeps the timeline at the requested position for up to 3 s, until the app reports it, so the timeline doesn't snap back.
- `Playback::position_now()` extrapolates from the last reading while playing (Spotify reports position only on play, pause and seek).
- The main thread detects changes through `versions()` and refreshes `track`, sets album art in the UI, picks the palette target, shows the toast, and runs scene auto-switch on a track change (but not for the first track seen).

### 3.7 UI (`src/ui/`)

The UI is **egui**, in immediate mode, redrawn every frame it runs.

- **Contract**: `UiState::draw(ctx, UiCtx) -> (Vec<UiAction>, Rect)`. `UiCtx` carries `&mut Settings` (edited in place) plus read-only state: track, vis, palette, mode, stats, fade alphas, update status, and so on. Anything that isn't a setting is requested with a `UiAction` (`SetMode`, `HideSidebar`, `Media(MediaCmd)`, `Update(UpdateCmd)`, `Restart`) and applied by `App::frame` after rendering. The UI never holds references to `App`, `Gpu` or workers, except `&Volume`, whose setters only send on a channel.
- The returned `Rect`, in points, is the area left for the scene once the side panels have taken their space.

| File | Contents |
|---|---|
| `mod.rs` | `UiAction`, `Section` + `SECTIONS`, `UiState`, `UiCtx`, `draw()` (panel layout, stacking of on-scene elements) |
| `theme.rs` | `Tokens` (neutral colours per `UiTheme`), `tokens()`, `with_alpha`, `to_c32` (linear → sRGB `Color32`), `UiState::style` (egui style, rebuilt only when the theme changes) |
| `widgets.rs` | `knob` (slider with a label row, reset-to-default and double-click reset), `choice` (segmented control with label and reset), `segmented`, `heading`, `nav_item`, `icon_button`, `spectrum_meter`, `Fmt` / `num()` / `PCT` value formatting |
| `icons.rs` | `Icon` enum + `paint_icon` (about 16 pt line icons drawn with egui shapes) |
| `playback.rs` | `seek_bar` (sidebar click-to-jump, or on-scene drag/scroll scrubbing), `time_bubble`, `transport`, `fmt_time` |
| `sidebar.rs` | Logo, section list, collapse/hide, now-playing card (art, title, seek bar, transport, spectrum) |
| `drawer.rs` | Drawer shell, Visuals footer (Randomize / Undo / Reset, undo depth 32), and one function per section: `visuals`, `audio`, `palette`, `general` (method; owns the reset confirm), `hotkeys`; also `palette_row`, `key_row`, `theme_card` |
| `overlay.rs` | `stats_hud`, `toast`, `window_buttons`, `volume_bar` (uses `wake::WakeSlider`), `timeline` |

UI conventions:

- **Colours come from `Tokens`.** Don't hard-code greys. Album colours appear only as data (spectrum, swatches, the accent on the volume-bar crest). Elements floating over the scene use `ft` (forced dark tokens) even in the light theme, because they sit on dark visuals.
- Every numeric setting goes through `knob` and every enum setting through `choice`, with the default taken from `Settings::default()` (or the scene settings' `Default`). That's what gives per-setting reset, default tooltips and consistent formatting. Use `PCT` for 0..1 values and `num(decimals, " unit")` otherwise, adding `.log()` for logarithmic ranges.
- An `egui::Area` remembers its size from earlier frames. If its content width changes (a new track title, say), give it an id that changes with the content. The toast does this.
- The drawer is a fixed 300 pt wide. Controls must fit: `knob` sizes its slider to the row width minus a 72 pt value box. Keep labels short.
- On-scene controls set `self.controls_hot` while hovered or dragged, so they don't fade out from under the pointer.
- Panels are opaque.
- Child modules of `ui` can read `UiState`'s private fields. Functions shared between siblings are `pub(super)`. Only `widgets::{knob, choice, num, PCT, Fmt}` and `Tokens` are public outside `ui`, because scenes use them.

### 3.8 Settings (`settings.rs`)

- `Settings` is one `#[serde(default)]` struct. A missing field takes its default, and unknown fields are ignored. A file that fails to parse as a whole falls back to all defaults; that's the existing behaviour.
- Scene-specific settings live in `settings.scenes: SceneSettings`, one field per scene (`ascii`, `scope`, `ball`), each that scene's own `Settings` struct. In JSON they appear as `"scenes": { "ascii": {..}, "scope": {..}, "ball": {..} }`.
- `load()` parses into `serde_json::Value`, runs `migrate()`, then deserializes. `migrate()` currently moves legacy flat `ascii_*` and `scope_*` keys into `scenes`. Unit tests next to it cover migration and round-tripping.
- **Stored names are an API.** The field names and enum variant names in `Settings`, `SceneSettings`, scene `Settings` structs and `SceneId` are what users' files contain. Renaming one silently resets that value for everyone unless you add a migration or `#[serde(alias = "...")]`. There's an example of the alias approach on `scope::Mode::Yt`.
- "The look" (what Randomize, Reset and Undo touch) is `Settings::copy_look` and `randomize_visuals`: every scene's settings plus the post effects (intensity, bloom, exposure, chromatic, grain, vignette). The current scene and render scale are deliberately excluded.
- "Reset all settings" (General) keeps theme, start mode, the Updates choice, sidebar size and window geometry.

### 3.9 Updates (`update.rs`)

- `settings.updates` (`UpdateMode::{Off, Notify, Auto}`, default Auto) is read once at launch. Unless it's Off, the worker waits 5 s and asks `api.github.com/repos/blaze280599/BlazeAudio/releases/latest`. With Auto it then installs what it finds. Check now and Install in General send `UpdateCmd`s whatever the mode.
- **Install**: download the exe and its `.sha256` asset, check the hash (via WinRT `HashAlgorithmProvider`) and the `MZ` header, write `<exe>.new`, rename the running `<exe>` to `<exe>.old`, rename `.new` to `<exe>`. If the second rename fails the first is undone. The worker deletes `.old` and any stray `.new` on the next start, retrying for a few seconds while a just-closed copy exits.
- The checksum catches broken downloads only. Without code signing nothing proves who built the release, so the security of updates is the security of the GitHub account.
- **Restart now** (`UiAction::Restart`) saves settings, starts the exe on disk with `--relaunch` and exits.
- A folder the user can't write to (Program Files, say) makes Install fail with a message telling them to move the exe.
- Files downloaded by the app carry no Mark of the Web, so an updated exe doesn't trigger SmartScreen again; only the user's first browser download does.

---

## 4. Code conventions

Match the surrounding code; it's consistent.

- **Comments**: every module starts with a `//!` paragraph saying what it is. Public items and non-obvious private ones get a one- or two-line `///` explaining *why* or *what for*, in plain prose (no Markdown headings, no `@param`). Inline comments explain intent or a non-obvious constraint ("Loopback routinely flags Xrun…"), never the mechanics of the next line. Don't leave commented-out code, TODO lists or change-log comments.
- **Line length**: long lines (up to about 150 characters) are normal here, and small struct literals and closures stay on one line. rustfmt isn't configured or enforced, so don't run `cargo fmt` over files you didn't otherwise change: it would rewrap large amounts of code.
- **Naming**: short local names in math-heavy code (`t`, `p`, `w`, `dt`, `k`) are fine and match the shaders. Types and public functions use full words.
- **Errors**:
  - User-facing startup failures go through `fatal()` (a message box) and then exit.
  - Worker failures are retried with sleeps.
  - Optional OS features degrade silently (`let _ =`, `.ok()`, `Option` chains).
  - `expect()` is reserved for true invariants (bundled font, thread spawn).
  - There's no `anyhow` or `thiserror`. Don't add them.
- **Dependencies**: keep the list short, and add `default-features = false` where possible. Prefer the existing crates (`windows`, `wgpu`, `egui`, `cpal`, `realfft`, `image`, `serde_json`, `bytemuck`, `ab_glyph`). Adding a crate is a product decision (binary size, build time), so ask first.
- **Unsafe**: only around Win32/COM calls, kept local and small.
- **GPU structs**: `#[repr(C)]`, `#[derive(Clone, Copy, Pod, Zeroable)]`, f32 fields, explicit `_pad` fields, and size a multiple of 16. Each one has a "Must match … in <file>.wgsl" doc line.
- **Frame-rate independence**: smoothing uses `1 - exp(-dt / tau)`, never fixed per-frame factors. `dt` is clamped to 0.1 s.
- **Commits** (when the user asks for one): the subject is imperative and capitalized, with no prefix and no trailing period ("Add track seeking: on-scene timeline, clickable sidebar bar, arrow keys"). A body explains what changed and why, in prose. One logical change per commit.

---

## 5. Recipes

### 5.1 Add a scene

The checklist also lives in the doc comment at the top of `src/scenes/mod.rs`. Keep both in sync.

**1. Shader** – create `src/shaders/<name>.wgsl`. Begin with a comment block in the style of the others: the scene's name in quotes, what it looks like, and what each audio feature drives. Write a fragment entry point `fs_<name>(in: VsOut) -> @location(0) vec4f`, which outputs linear HDR colour (values above 1 bloom). Use `G` for time, envelopes and palette, and `A.bands` / `band(x)` / `A.wave` for audio. If the scene has its own values:

```wgsl
struct Params {
  speed: f32, size: f32, mode: f32, _pad0: f32,
  // vec4/array members must start on a 16-byte boundary
  tint: vec4f,
};
@group(0) @binding(3) var<uniform> P: Params;
@group(1) @binding(0) var my_tex: texture_2d<f32>;   // only if the pass reads a texture
```

**2. Module** – create `src/scenes/<name>.rs`.

```rust
//! <Scene name>: one-line description of how it renders.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use super::{Params, Scene, SceneConfig, SceneFrame, SceneSettings};
use crate::gpu::{self, GpuCtx, HDR_FORMAT};
use crate::ui::widgets::{choice, knob, num, PCT};
use crate::ui::Tokens;
use crate::vis::VisState;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub speed: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { speed: 1.0 }
    }
}

impl SceneConfig for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let d = Self::default();
        knob(ui, t, "Speed", &mut self.speed, d.speed, 0.1..=4.0, num(2, "×"));
    }

    fn randomize(&mut self, r: &mut dyn FnMut(f32, f32) -> f32) {
        self.speed = r(0.5, 2.0);
    }
}

/// Must match `Params` in <name>.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MyParams {
    speed: f32,
    _pad: [f32; 3],
}

pub struct MyScene {
    pipe: wgpu::RenderPipeline,
}

impl MyScene {
    pub fn new(gpu: &GpuCtx) -> Self {
        let m = gpu.module("<name>", include_str!("../shaders/<name>.wgsl"));
        Self { pipe: gpu.fullscreen(&gpu.layout(false), &m, "fs_<name>", HDR_FORMAT, None) }
    }
}

impl Scene for MyScene {
    fn params(&mut self, _vis: &VisState, settings: &SceneSettings, p: &mut Params) {
        p.set(&MyParams { speed: settings.my_scene.speed, _pad: [0.0; 3] });
    }

    fn encode(&mut self, enc: &mut wgpu::CommandEncoder, f: &SceneFrame) {
        gpu::fullscreen_pass(enc, f.target, wgpu::LoadOp::Clear(wgpu::Color::BLACK), &self.pipe, f.g0, None);
    }
}
```

`GpuCtx` gives you:

- `module(name, src)`: common.wgsl is prepended.
- `layout(tex1)`: group 0, plus a one-texture group 1 when `tex1` is true.
- `fullscreen(layout, module, fs, format, blend)`: a full-screen-triangle pipeline.
- `tex1(view)`: a group 1 bind group.
- `device`, `queue`, `layouts` for anything custom.

`gpu::make_tex` creates a render-target-and-sampled texture, and `gpu::fullscreen_pass` runs one full-screen pass.

Variations:

- **No settings**: skip the `Settings` struct and the `ui` imports, and don't override `params()`. `terminal.rs` is the 21-line example.
- **A texture built once** (atlas, LUT): build it in `new()` and keep the `wgpu::Texture` alive in a field (`_atlas`), along with its bind group. See `ascii.rs`.
- **Size-dependent textures or feedback** (trails, afterglow, reaction-diffusion): take `size` in `new(gpu, size)`, rebuild in `resize()`, ping-pong two textures, and clear history when `f.fresh`. Use a compact format such as `R16Float` when you only need intensity. See `scope.rs`.
- **Geometry instead of a full-screen pass**: build a custom `RenderPipeline` with `gpu.device` and `gpu.layout(..)`, and draw instanced quads from `A.wave` or `A.bands` in the vertex shader. Again, `scope.rs` (`vs_beam`) is the example.
- **Per-instance data the vertex shader reads** (particle tables): group 1's shared layouts are fragment-only textures, so make your own bind group layout with `VERTEX` visibility (a read-only storage buffer works well) and a pipeline layout of `[&gpu.layouts.g0, &yours]`. See `ball.rs` (dust).
- **Constant tables too big for `Params`**: generate them as WGSL `var<private>` arrays in Rust and prepend them to the shader source before `gpu.module`. See `ball.rs` (`strand_tables`).
- **Scene-specific analysis** (zero-crossing triggers, peak picking) belongs in the scene's `params()`, reading `vis.wave`, `vis.bands`, `vis.dt` and the other public fields. It doesn't belong in `VisState`. If it needs state between frames, keep it on the scene struct.

**3. Register** in `src/scenes/mod.rs`:

- `mod <name>;` (or `pub mod` if `snapshot.rs` needs its types)
- a `SceneId::<Name>` variant, appended to `SceneId::ALL`. The order of `ALL` is the order of the buttons and number keys; append rather than insert unless the user asks.
- an arm in `SceneId::name()` with the display name, and one in `SceneId::build()`
- if it has settings: a field `pub <name>: <name>::Settings` in `SceneSettings`, and an arm in `SceneSettings::config()`

From those, everything else updates automatically: sidebar buttons, number keys (first 9), `S` / Next, auto-switch (by track and timer), Randomize (through `SceneSettings::randomize` → `config()`), Reset and Undo (through `copy_look`, which copies all of `scenes`), and the Hotkeys list.

**4. Self-test** – add a block to `snapshot::run` after the existing scenes:

- set `gpu.scene` and `settings.visual`, and reset `vis.scene_time`
- time 150 frames and append `render (<name>): … ms/frame` to the report
- capture one PNG per mode or shape

**5. Docs** – in `README.md`, add the scene to the scene list and the `1` / `2` / `3` row of the Controls table. Update §1 of this file (the list of current scenes).

**6. Verify** – `cargo clippy --all-targets` (no new warnings in your files), `cargo test`, then the snapshot. Look at the new PNGs, and byte-compare the existing ones against the previous build; they must not change. Check the ms/frame for the new scene: about 1.2 ms at 1280×720 is typical, and much more than that needs a reason.

**Shader quality bar** – scenes run at up to 170 Hz at 1440p on the user's machine, and on weaker GPUs at lower `render_scale`.

- Keep per-pixel loops bounded and small.
- Never produce NaN or Inf. Bloom clamps them, but they still show as dark or bright pixels. Guard `pow`, `sqrt`, `log`, division and `normalize`.
- Respect `G.intensity` as a global brightness scale.
- Let silence look calm: envelopes decay to zero, and `VisState` already fades `A.wave` out when the capture stops, so test the scene with silence too (`BLAZE_IGNORE_AUDIO=1`).

### 5.2 Add a setting

- **Scene-specific**:
  1. Add the field to the scene's `Settings` and its `Default`.
  2. Add a `knob` or `choice` in its `SceneConfig::ui()`, and a line in `randomize()` if it's part of the look.
  3. Pass it through the scene's `Params` (Rust struct, WGSL struct, and `params()`; mind the padding).
- **App-wide**:
  1. Add the field to `Settings` and its value in `Default for Settings`.
  2. Add the control to the right function in `ui/drawer.rs`.
  3. Read it where it's needed. For example, analyzer settings go through `audio_params()` in `main.rs`, which is pushed to the audio thread while the drawer is open.
  4. If it's part of the look, add it to `Settings::copy_look` and `randomize_visuals`.
- **A value every shader needs**: add it to `gpu::Globals` **and** `Globals` in `common.wgsl`, at the same position. Replace the `_pad` / `_pad0` slot if one is free, or add a full vec4's worth of fields so the size stays a multiple of 16 and `pal` stays 16-byte aligned. Fill it in `VisState::globals()`. Prefer a scene's `Params` unless several scenes need the value.
- **Renaming, moving or retyping a stored setting**: add a rewrite in `settings::migrate()` (or a serde alias for a simple rename) and a unit test next to the existing ones.

### 5.3 Add a hotkey

1. Add an arm to `App::handle_key` in `main.rs`. Keys arrive only without Ctrl or Alt.
2. Add a row to the `rows` array in `hotkeys()` (`ui/drawer.rs`), and bump the array length.
3. Add the key to the Controls table in `README.md`.

### 5.4 Add a settings section

1. Add a `Section` variant and an entry in `SECTIONS` (`ui/mod.rs`) with a label and an `Icon`. Add the icon's variant and its drawing in `ui/icons.rs` (fits a 16 pt box, 1.5 pt strokes, `col` colour).
2. Add an arm in `drawer_body()` (`ui/drawer.rs`) that calls a new section function.
3. If the section needs state that isn't in `Settings`, add it to `UiCtx` and fill it in `App::frame`.

### 5.5 Add something on the scene

1. Draw it in `ui/overlay.rs`: an `egui::Area` anchored to the `scene` rect, `ft` tokens, and `ui.multiply_opacity(alpha)`.
2. Call it from `UiState::draw`, respecting the bottom-centre stack (volume bar, timeline, toast lift).
3. Compute its alpha in `App::frame`, usually from `controls_alpha`, pass it through `UiCtx`, and add it to **both** `need_ui` and `idle`.
4. If the pointer can rest on it, set `self.controls_hot` so it doesn't fade out. Give it an on/off toggle under General → On the scene.

### 5.6 Ask the app to do something from the UI

Add a `UiAction` variant, push it from the UI code, and handle it in the `for a in actions` loop near the end of `App::frame`. Don't give the UI new references to app internals.

### 5.7 New audio features

1. Compute the feature in `Analyzer::process` (`audio.rs`) at hop rate, and add it to `AudioFrame` and its `Default`.
2. Smooth it in `VisState::update` with dt-based envelopes. Treat a stale frame (no packet for 150 ms) as silence.
3. Expose it through `Globals` (everyone) or a scene's `Params` (one scene).
4. Run the snapshot. `report.txt` shows whether beat detection still matches the synthetic 124 BPM track.

Keep the capture callback cheap: no allocation or locking beyond the existing per-hop publish.

### 5.8 Cut a release

1. Bump `version` in `Cargo.toml` (semver; the updater compares `major.minor.patch`), then `cargo build` so `Cargo.lock` follows, and commit both.
2. Tag and push: `git tag v1.2.0 && git push origin main v1.2.0`. The tag must be the version with a `v` in front; the workflow refuses a mismatch.
3. `.github/workflows/release.yml` runs the tests, builds the release exe, writes `blazeaudio.exe.sha256`, and publishes a release with both files and generated notes. Every running copy set to Notify or Auto sees it on its next launch.
4. Don't mark a release you want installed as a prerelease: `releases/latest` skips prereleases.

---

## 6. Verification checklist (before saying "done")

1. `cargo check` and `cargo clippy --all-targets`: nothing new in the files you touched.
2. `cargo test` passes. Add tests for pure logic you introduce, such as settings migration or maths helpers.
3. `cargo run -- --snapshot <dir>` succeeds. For changes that shouldn't affect the visuals, the PNGs are byte-identical to the previous build. For visual changes, you've looked at the PNGs. Frame times and beat stats are in line.
4. For UI, window-mode or input changes, the snapshot can't see them. Launch the app with a scratch `APPDATA`, confirm it starts and closes cleanly, and tell the user exactly what you couldn't check by eye, so they can click through it.
5. Docs are updated: `README.md` for user-visible behaviour, this file for anything about architecture, conventions or recipes.

Report what you verified and what you didn't. Don't claim a UI change works because it compiled.

---

## 7. Performance reference

Release build, RTX 5070 Ti, 2560×1440 at 170 Hz:

| State | CPU (whole machine, 16 threads) | Working set | Private |
|---|---|---|---|
| Idle (no audio) | 0.10% | 18 MB | 9 MB |
| Playing, 170 fps | 0.64% | 26 MB | 13 MB |

- The self-test gives about 1.2 ms per frame per scene (CPU + GPU, 1280×720, dev build). Init including shader compilation takes about 0.5 s.
- Process commit (Private Bytes) shows about 215 MB, almost all of it address space reserved by the D3D12 driver and wgpu's descriptor heap, not resident memory. Don't "fix" it.
- Hot paths:
  - **every frame**: `App::frame`, `VisState::update`, `globals()`, `params()`, `encode()`, and the UI when visible
  - **per hop (~5 ms)**: `Analyzer::process`

  Avoid heap allocation, string formatting and locks there, beyond what already exists. `Params` is a fixed 256-byte stack buffer for this reason.

---

## 8. Where things are (quick index)

| I need to… | Look at |
|---|---|
| Change what a scene looks like | `src/shaders/<scene>.wgsl` |
| Change a scene's controls or defaults | `src/scenes/<scene>.rs` (`Settings`, `SceneConfig::ui`) |
| Change bloom, tonemap, grain or vignette | `src/shaders/post.wgsl`, settings in `Settings`, values in `VisState::globals` |
| Change how music drives the visuals (smoothing, kick envelope, flow clock) | `src/vis.rs` |
| Change beat detection, bands or tempo | `src/audio.rs` (`Analyzer`) |
| Change album-palette extraction or presets | `src/palette.rs` |
| Change now-playing, art fetch or transport | `src/media.rs` |
| Change volume control | `src/volume.rs` (worker), `ui/overlay.rs` `volume_bar`, `wake.rs` (slider widget) |
| Change window modes, keys or frame pacing | `src/main.rs` |
| Change CPU/RAM stats or working-set trimming | `src/sys.rs` |
| Change update checks, download or the exe swap | `src/update.rs`, the release workflow in `.github/workflows/release.yml` |
| Change the sidebar or drawer | `src/ui/sidebar.rs`, `src/ui/drawer.rs` |
| Change UI colours or the egui style | `src/ui/theme.rs` |
| Change the self-test | `src/snapshot.rs` |

---

## 9. Pitfalls (things that have bitten, or will)

- **WGSL ⇄ Rust layout drift.** `Globals` and every `Params` exist twice. A mismatch doesn't error; it just feeds garbage into the shader. Change both sides together, keep vec4s and arrays 16-byte aligned, and keep the "Must match" doc comment.
- **FXC**: the DX12 backend compiles through FXC (HLSL SM5), which is slower and stricter than DXC. Very long unrolled loops or huge shaders can take seconds to compile or fail. Watch the "init" line in `report.txt` when adding heavy shaders.
- **Uniform size**: `Params` is capped at 256 bytes (`PARAMS_BYTES`); `Params::set` asserts. For more data, use a storage buffer or texture owned by the scene in group 1.
- **Name collisions** with `common.wgsl`: a scene shader can't define `G`, `A`, `P` (except its own `Params`), `vs_full`, `VsOut`, `hash11`, `band`, `pal`, `luma`, `crt_curve` and so on.
- **`SceneId::ALL` order** sets the number keys and the startup default (`ALL[0]`). Reordering changes what users' keys do.
- **egui `Area` sizing**: an Area keeps its previous size, so content that grows wraps or clips unless the Area's id changes.
- **Main-thread blocking**: a COM or WinRT call on the main thread (media control, volume) can stall frames for tens of milliseconds. Always go through the worker's channel.
- **`panic = "abort"`** in release: a panic anywhere ends the process. Prefer `Option` and `let Some(..) else` over `unwrap()` in new code.
- **Settings compatibility**: renaming a field, enum variant or `SceneId` resets users' saved values unless you migrate (§3.8).
- **Running the app touches real state**: the settings file, the system volume (if you exercise the volume bar), and the user's media playback (transport commands). Use a scratch `APPDATA`, and don't send media or volume commands without the user's say-so.
- **The updater and dev builds**: dev builds skip the launch check (unless `BLAZE_UPDATE_URL` is set), so running from `target/` never replaces that exe unasked. Check now and Install still work in a dev build and *will* replace the exe you're running, so test installs on a copy in a scratch folder.
- **Release asset names are an API**: the updater looks for `blazeaudio.exe` and `blazeaudio.exe.sha256` (`<hex>  blazeaudio.exe`) on the latest non-prerelease release, and compares its `vX.Y.Z` tag with `CARGO_PKG_VERSION`. Renaming either asset or the repo breaks updates for every installed copy.
- **Line endings**: the repo stores LF, with `core.autocrlf=true` on this machine. Git's LF→CRLF warnings on commit are expected and harmless.

---

## 10. Keeping this file current

Update this file in the same change when you:

- add, remove or rename a module, scene, setting group, thread, binding or `UiAction`
- change a convention, constraint or the verification steps
- discover a new pitfall

Keep the recipes runnable: if you change a trait signature in `scenes/mod.rs`, update the §5.1 example and the checklist in that file's doc comment.
