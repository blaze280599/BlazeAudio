# BlazeAudio

An HDR music visualizer for Windows. It captures whatever your PC is playing through WASAPI loopback, reads now-playing info and album art from Spotify (or any media app) through Windows' media session API, and renders eleven audio-reactive scenes with DirectX 12.

Everything ships in one self-contained `blazeaudio.exe` (static CRT, system DLLs only).

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

Scenes crossfade with a noisy radial dissolve. Auto-cycle switches scenes on the beat, and a big drop can also trigger a switch.

## Controls

| Key | Action |
|-----|--------|
| `F1` / `Tab` | Settings overlay |
| `F11` / `Alt+Enter` / double-click | Borderless fullscreen |
| `O` | Overlay mode (transparent, always-on-top, click-through) |
| `←` `→` / `1`–`9`, `0`, `-` / `Space` | Change scene |
| `T` | Show now-playing card |
| `F3` | FPS / CPU / RAM / BPM stats |
| `Esc` | Close settings → leave fullscreen/overlay → quit |
| `Ctrl+Alt+O` | Toggle overlay (**global**, works while click-through) |
| `Ctrl+Alt+S` | Toggle settings (**global**, makes the overlay clickable while open) |
| `Ctrl+Alt+N` | Next scene (**global**) |

Settings are saved to `%APPDATA%\BlazeAudio\settings.json`.

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
  - Bloom is Jimenez's 13-tap downsample with a Karis average and soft-knee threshold, plus a tent upsample over 6 mips.
  - Final output is fitted ACES, chromatic aberration on kicks, vignette, and film grain/dither applied in display space.
- **Overlay transparency** uses a DirectComposition swapchain (`DxgiFromVisual`) with premultiplied alpha.
- **Idle**: after 3 s of silence (and when nothing on screen needs it) the frame rate drops to the idle cap (default 30 fps), egui stops running, and nothing renders while the window is minimized or occluded.

## Performance (RTX 5070 Ti, 2560×1440 @ 170 Hz, release build)

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

This synthesizes a 124 BPM track, runs it through the real analyzer, renders every scene and a crossfade offscreen to PNGs, and writes `report.txt` with timings, detected beats and their phase, and the BPM estimate.

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
  gpu.rs        wgpu renderer: targets, pipelines, bloom, tonemap, egui
  vis.rs        render-side animation state, scene sequencing
  ui.rs         egui settings panel, now-playing toast, stats
  sys.rs        global hotkeys, CPU/RAM stats, working-set trim
  snapshot.rs   headless self-test
  shaders/      one WGSL file per scene, plus common and post
```
