# BlazeAudio

An HDR music visualizer for Windows. It captures whatever your PC is playing through WASAPI loopback, reads now-playing info and album art from Spotify (or any media app) through Windows' media session API, and renders audio-reactive visuals (Faulty Terminal and Gradient ASCII) with DirectX 12.

Everything ships in one self-contained `blazeaudio.exe` (static CRT, system DLLs only). There's nothing to install.

## Requirements

- Windows 10 or 11
- A DirectX 12 capable GPU

## Getting started

1. Run `blazeaudio.exe`.
2. Play music in Spotify, a browser, a game, or anything else. BlazeAudio visualizes everything your default output device plays.
3. If the app reports what's playing to Windows (Spotify does), BlazeAudio shows the title, artist, and album art, and takes its colors from the cover.

In a window, settings live in the sidebar on the left. Press `F1` to show or hide it.

## Faulty Terminal

A flickering CRT full of procedural glyphs. The columns form a spectrum analyzer mirrored around the center (glyphs light up to each band's level, with a bright cap on top), highs make the glyphs churn faster, and each beat tears rows sideways with an RGB-split glitch.

## Gradient ASCII

A soft light seen through a grid of monospaced characters (` =+xX0S8#@`), brighter cells using denser glyphs. Pick the light's shape in **Visuals**: Horizon (a glow over a dark curve), Pinch (a band squeezed in the middle), Rise (a glow coming up from the bottom) or Eclipse (a corona around a dark disc). The edge of the light follows the spectrum and swells with the music, hats make the characters flicker, and each beat sends a ring of scrambled characters out from the light. When the scene comes on, the light fades in from its source while the characters decode.

Press `1` / `2` or use the buttons at the top of **Visuals** to switch scenes.

## Album-art colors

When the track changes, BlazeAudio extracts the main colors from the cover and fades the visualizer to them over about a second. A now-playing card with the cover, title, and artist appears for a few seconds; press `T` to show it again.

With nothing playing, or with album colors turned off, it uses one of five built-in palettes: Neon Synth, Ember, Aurora, Glacier, or Acid.

## Window modes

- **Windowed**: a normal resizable window (the default).
- **Fullscreen**: borderless fullscreen on the monitor the window is on. The cursor hides after 2.5 s without movement.
- **Overlay**: covers the monitor with a transparent background, so only the glowing parts show over your desktop and apps. It stays on top and clicks pass through to whatever is underneath.

To keep it on a second monitor while you game or work, drag the window there and press `F11`, or use **Window → Start in** in settings to choose the startup mode.

## Controls

| Key | Action |
|-----|--------|
| `F1` / `Tab` | Show / hide the sidebar |
| `F11` / `F` / `Alt+Enter` / double-click | Borderless fullscreen |
| `O` | Overlay mode |
| `1` / `2` | Faulty Terminal / Gradient ASCII |
| `T` | Show now-playing card |
| `F3` | FPS / CPU / RAM / BPM stats |
| `Esc` | Close the settings drawer, then the sidebar (fullscreen/overlay), then leave fullscreen/overlay, then quit |
| `Ctrl+Alt+O` | Toggle overlay (**global**, works while click-through) |
| `Ctrl+Alt+S` | Toggle the sidebar (**global**, makes the overlay clickable while open) |

The two `Ctrl+Alt` shortcuts work even when BlazeAudio isn't focused, which you need in overlay mode because the window can't receive normal key presses there.

## Sidebar

In windowed mode a sidebar sits on the left, and the visualizer renders in the space beside it. It shows what's playing, with a live spectrum meter, plus one entry per settings section. Click a section to open its settings in a drawer next to the sidebar; click it again or press `Esc` to close the drawer. **Collapse** shrinks the sidebar to an icon rail, and the app remembers that choice.

Fullscreen and overlay hide the sidebar so the visuals get the whole screen; press `F1` to bring it back. Moving the mouse over the visualizer shows fullscreen/overlay buttons in the top-right corner, which fade out after a couple of seconds.

The sections:

- **Visuals**: scene, Gradient ASCII shape, cell size, density, glyph and gradient opacity, intensity, bloom strength and threshold, exposure, chromatic aberration, film grain, vignette, render scale (lower it on slower GPUs).
- **Audio**: sensitivity, attack (how fast bars rise), decay (how fast they fall), beat sensitivity.
- **Palette**: album-art colors on/off, fallback palette, swatches of the current colors.
- **Window**: mode, overlay opacity, overlay click-through, startup mode, idle frame-rate cap, now-playing card, performance stats.
- **Theme**: Dark, OLED black, or Light for the sidebar and controls.
- **Hotkeys**: the full key list.
- **Reset to defaults**.

Settings are saved to `%APPDATA%\BlazeAudio\settings.json` when you close the panel or quit.

## How it works

- **Capture**: cpal builds an *input* stream on the default *output* device, which is WASAPI loopback. The stream is rebuilt automatically when the default device changes.
- **Analysis** runs in the capture callback at a fixed 256-sample hop (~5 ms), so it doesn't depend on frame rate and costs nothing when audio is silent:
  - Blackman-Harris window, 2048-point realfft
  - 64 log-spaced bands from 30 Hz to 16 kHz with +3 dB/oct pink compensation and fast-attack/slow-release AGC
  - Per-band attack/decay smoothing (configurable)
  - **Beats**: SuperFlux-style spectral flux (lag-4, max-filtered, kick-weighted) with an adaptive mean + kσ threshold
  - **Tempo**: autocorrelation of the onset envelope over 5.5 s with a log-normal 120 BPM prior. Once a tempo locks, a refractory window keeps beat pulses on the beat instead of off-beat hats.
- **Metadata**: `GlobalSystemMediaTransportControlsSessionManager` (prefers a Spotify session). The cover thumbnail is decoded, then a palette comes from k-means in Oklab, scored for vividness and hue diversity. The Spotify Web API is not used.
- **Rendering** (wgpu → DX12):
  - The scene draws into an Rgba16Float target sized to the area beside the sidebar.
  - It's driven by smoothed bands, kick/hat envelopes, the beat count, and a "flow" clock that idles slowly and surges with the music.
  - Bloom clamps NaN and infinite values out of the scene, so a single bad pixel can't be smeared across the frame.
  - Bloom is Jimenez's 13-tap downsample with a Karis average and soft-knee threshold, plus a tent upsample over 6 mips.
  - Final output is fitted ACES, chromatic aberration on kicks, vignette, and film grain/dither applied in display space.
- **Overlay transparency** uses a DirectComposition swapchain (`DxgiFromVisual`) with premultiplied alpha.
- **Idle**: after 3 s of silence (and when nothing on screen needs it) the frame rate drops to the idle cap (default 30 fps), egui stops running, and nothing renders while the window is minimized or occluded.

## Performance (RTX 5070 Ti, 2560×1440 @ 170 Hz, release build)

CPU and memory, measured on the earlier multi-scene build (Halo scene):

| State | CPU (whole machine, 16 threads) | Working set | Private working set |
|-------|------|------|------|
| Idle (no audio) | 0.10 % | 18 MB | 9 MB |
| Playing music, 170 fps | 0.64 % | 26 MB | 13 MB |

The self-test measures both scenes at about 1.1 ms per frame (CPU + GPU, 1280×720, debug build). Shader compilation plus device init takes about 0.4 s.

Process *commit* (Private Bytes) reads about 215 MB. Nearly all of that is reserved, untouched address space from the D3D12 driver and wgpu's fixed 1M-entry descriptor heap. It isn't resident RAM. The "Memory" column in Task Manager shows the private working set.

## Building

Requires Rust (stable, MSVC toolchain) and the Visual Studio C++ build tools.

```bash
cargo build --release
```

The output is `target\release\blazeaudio.exe`. `.cargo/config.toml` enables `+crt-static`, so no VC++ redistributable is needed.

### Self-test

```bash
target/release/blazeaudio.exe --snapshot out
```

This runs headless: no window opens. It synthesizes a 124 BPM track, runs it through the real analyzer, and renders the visualizer offscreen to PNGs: Faulty Terminal on a beat and between beats, then Gradient ASCII in each of its four shapes. It also writes:

- `report.txt`: init time, frame time, detected beats with their phase, and the BPM estimate
- `onset_trace.csv`: the onset and threshold curve from the audio-only check

The output folder defaults to `snapshots` if you leave it off.

### Diagnostics

- `BLAZE_IGNORE_AUDIO=1`: render as if silent, to measure the idle path.
- `BLAZE_TRACE=<file.csv>`: dump ~10 s of the live onset/threshold curve.

## Layout

```
src/
  main.rs       winit app, window modes, hotkeys, frame pacing
  audio.rs      WASAPI loopback, FFT, bands, flux beats, tempo
  media.rs      GSMTC now-playing + album art
  palette.rs    Oklab k-means album palette, presets
  gpu.rs        wgpu renderer: scene target, bloom, tonemap, egui
  vis.rs        render-side animation state: bands, kick, flow clock, palette fade
  ui.rs         egui sidebar + settings drawer, themes, now-playing toast, stats
  settings.rs   settings model and persistence
  sys.rs        global hotkeys, CPU/RAM stats, working-set trim, overlay window fix
  snapshot.rs   headless self-test
  shaders/      terminal.wgsl, ascii.wgsl (the scenes), common.wgsl, post.wgsl (bloom + tonemap)
```

## Credits

Faulty Terminal and Gradient ASCII are inspired by the [React Bits](https://reactbits.dev) backgrounds of the same names. Faulty Terminal is written from scratch for wgpu/WGSL; Gradient ASCII is ported from the React Bits WebGL2 shader. Both are driven by audio instead of the cursor. The glyphs use Hack, the monospace font bundled with egui.
