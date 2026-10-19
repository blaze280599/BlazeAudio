# BlazeAudio

An HDR music visualizer for Windows. It captures whatever your PC is playing through WASAPI loopback, reads now-playing info and album art from Spotify (or any media app) through Windows' media session API, and renders eleven audio-reactive scenes with DirectX 12.

Everything ships in one self-contained `blazeaudio.exe` (static CRT, system DLLs only). There's nothing to install.

## Requirements

- Windows 10 or 11
- A DirectX 12 capable GPU

## Getting started

1. Run `blazeaudio.exe`.
2. Play music in Spotify, a browser, a game, or anything else. BlazeAudio visualizes everything your default output device plays.
3. If the app reports what's playing to Windows (Spotify does), BlazeAudio shows the title, artist, and album art, and takes its colors from the cover.

Press `F1` for settings and `←` / `→` to change scenes.

## Scenes

| Key | Scene | What reacts |
|-----|-------|-------------|
| `1` | **Halo** | 128 mirrored log-frequency bars around the album art, falling peak caps, oscilloscope ring, beat shockwaves, god rays |
| `2` | **Nebula** | 131,072 compute-shader particles spring toward morphing shapes (spectral sphere → torus → galaxy → cymatic plane), stirred by a divergence-free flow field, with HDR trails and radial bursts on kicks |
| `3` | **Silk** | Satin folds under a moving light: bass deepens the folds, mids turn the light, highs sharpen the sheen, beats ripple the cloth |
| `4` | **Plasma** | Raymarched volumetric folds of light; energy speeds the turbulence, bass swells the shell |
| `5` | **Molten Metal** | Caustic filaments through a heat ramp that run white-hot on kicks, with heat waves on each beat |
| `6` | **Ferrofluid** | Glossy black sea of magnetic spikes; the spectrum raises spikes across the field and bass raises a crown |
| `7` | **Liquid Ether** | Real-time fluid simulation colored by flow speed, stirred by six band-driven emitters plus a burst on each kick |
| `8` | **Light Tunnel** | Fibre-optic strands spiralling into depth, with comets whose rate follows each strand's frequency band |
| `9` | **Scanner** | CRT of stacked spectrum ridge lines (newest in front), swept by a beam once per beat with phosphor afterglow |
| `0` | **Dither** | Domain-warped noise waves, Bayer-dithered into four palette shades, with beat rings |
| `-` | **Faulty Terminal** | Flickering glyph CRT whose columns form a spectrum analyzer, with row-tearing glitches on beats |

Scenes crossfade with a noisy radial dissolve. Auto-cycle (every 40 s by default) switches scenes on a beat, and a big drop can also trigger a switch.

## Album-art colors

When the track changes, BlazeAudio extracts the main colors from the cover and fades every scene to them over about a second. A now-playing card with the cover, title, and artist appears for a few seconds; press `T` to show it again.

With nothing playing, or with album colors turned off, it uses one of five built-in palettes: Neon Synth, Ember, Aurora, Glacier, or Acid.

## Window modes

- **Windowed**: a normal resizable window (the default).
- **Fullscreen**: borderless fullscreen on the monitor the window is on. The cursor hides after 2.5 s without movement.
- **Overlay**: covers the monitor with a transparent background, so only the glowing parts show over your desktop and apps. It stays on top and clicks pass through to whatever is underneath.

To keep it on a second monitor while you game or work, drag the window there and press `F11`, or use **Window → Start in** in settings to choose the startup mode.

## Controls

| Key | Action |
|-----|--------|
| `F1` / `Tab` | Settings panel |
| `F11` / `F` / `Alt+Enter` / double-click | Borderless fullscreen |
| `O` | Overlay mode |
| `→` / `N` / `Space` | Next scene |
| `←` / `P` | Previous scene |
| `1`–`9`, `0`, `-` | Jump to a scene |
| `T` | Show now-playing card |
| `F3` | FPS / CPU / RAM / BPM stats |
| `Esc` | Close settings, then leave fullscreen/overlay, then quit |
| `Ctrl+Alt+O` | Toggle overlay (**global**, works while click-through) |
| `Ctrl+Alt+S` | Toggle settings (**global**, makes the overlay clickable while open) |
| `Ctrl+Alt+N` | Next scene (**global**) |

The three `Ctrl+Alt` shortcuts work even when BlazeAudio isn't focused, which you need in overlay mode because the window can't receive normal key presses there.

## Settings panel

`F1` opens a panel showing what's playing, the active audio device, and a live spectrum meter, with these sections:

- **Scenes**: pick a scene, auto-cycle on/off, cycle time, switch on drops, crossfade length.
- **Visuals**: intensity, bloom strength and threshold, exposure, chromatic aberration, film grain, vignette, particle trails and density, render scale (lower it on slower GPUs).
- **Audio**: sensitivity, attack (how fast bars rise), decay (how fast they fall), beat sensitivity.
- **Palette**: album-art colors on/off, fallback palette, swatches of the current colors.
- **Window**: mode, overlay opacity, overlay click-through, startup mode, idle frame-rate cap, now-playing card, performance stats.
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
  - Each scene draws into an Rgba16Float target, with two ping-pong slots for crossfades.
  - Scenes are driven by smoothed bands, kick/hat envelopes, beat shockwaves, a 4-second spectrogram texture, the tracked tempo (beat phase), and a "flow" clock that idles slowly and surges with the music.
  - Nebula updates its particles in a compute shader. Liquid Ether runs a stable-fluids simulation on a 200-row grid each frame (advection, vorticity confinement, 24 Jacobi pressure iterations, projection).
  - The compositor clamps NaN and infinite values out of every scene, so a single bad pixel can't be smeared across the frame by bloom.
  - Bloom is Jimenez's 13-tap downsample with a Karis average and soft-knee threshold, plus a tent upsample over 6 mips.
  - Final output is fitted ACES, chromatic aberration on kicks, vignette, and film grain/dither applied in display space.
- **Overlay transparency** uses a DirectComposition swapchain (`DxgiFromVisual`) with premultiplied alpha.
- **Idle**: after 3 s of silence (and when nothing on screen needs it) the frame rate drops to the idle cap (default 30 fps), egui stops running, and nothing renders while the window is minimized or occluded.

## Performance (RTX 5070 Ti, 2560×1440 @ 170 Hz, release build)

CPU and memory, measured on the Halo scene:

| State | CPU (whole machine, 16 threads) | Working set | Private working set |
|-------|------|------|------|
| Idle (no audio) | 0.10 % | 18 MB | 9 MB |
| Playing music, 170 fps | 0.64 % | 26 MB | 13 MB |

GPU frame cost at 1280×720 is 0.4–1.4 ms per scene (Ferrofluid and Halo are the heaviest). Shader compilation plus device init takes about 0.6 s.

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

This runs headless: no window opens. It synthesizes a 124 BPM track, runs it through the real analyzer, and renders every scene (on a beat and between beats) plus a crossfade offscreen to PNGs. It also writes:

- `report.txt`: init time, per-scene frame time, detected beats with their phase, and the BPM estimate
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
  gpu.rs        wgpu renderer: targets, pipelines, fluid sim, bloom, tonemap, egui
  vis.rs        render-side animation state, fluid emitters, scene sequencing
  ui.rs         egui settings panel, now-playing toast, stats
  settings.rs   settings model and persistence
  sys.rs        global hotkeys, CPU/RAM stats, working-set trim, overlay window fix
  snapshot.rs   headless self-test
  shaders/      one WGSL file per scene, plus common and post
```

## Credits

Silk, Plasma, Molten Metal, Ferrofluid, Liquid Ether, Light Tunnel, Scanner, Dither and Faulty Terminal are inspired by the [React Bits](https://reactbits.dev) backgrounds of the same names. The shaders here are written from scratch for wgpu/WGSL and driven by audio instead of the cursor.
