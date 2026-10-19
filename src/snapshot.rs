//! `blazeaudio.exe --snapshot <dir>`: headless self-test. Synthesizes a 124 BPM track, runs it
//! through the real analyzer, renders the visualizer offscreen and writes PNGs + a beat report.

use std::path::Path;
use std::time::Instant;

use crate::audio::Offline;
use crate::gpu::Gpu;
use crate::palette::{self, PRESETS};
use crate::settings::Settings;
use crate::vis::VisState;

const SR: f32 = 48_000.0;
const BPM: f32 = 124.0;

struct Synth {
    t: f64,
    kick_phase: f32,
    bass_phase: f32,
    rng: u32,
    hp: f32,
}

impl Synth {
    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn sample(&mut self) -> f32 {
        let dt = 1.0 / SR as f64;
        let beat = 60.0 / BPM as f64;
        let tb = (self.t % beat) as f32;
        let bar_pos = ((self.t / beat) as u64) % 4;
        // Kick: pitch-swept sine.
        let f = 45.0 + 120.0 * (-tb * 30.0).exp();
        if tb < dt as f32 {
            self.kick_phase = 0.0;
        }
        self.kick_phase += f / SR;
        let kick = (self.kick_phase * std::f32::consts::TAU).sin() * (-tb * 7.0).exp() * 0.9;
        // Off-beat hats: high-passed noise.
        let th = ((self.t + beat * 0.5) % beat) as f32;
        let n = self.noise();
        let hpn = n - self.hp;
        self.hp = n;
        let hat = hpn * (-th * 45.0).exp() * 0.22;
        // Snare on 2 and 4.
        let snare = if bar_pos % 2 == 1 { self.noise() * (-tb * 18.0).exp() * 0.3 } else { 0.0 };
        // Rolling saw bass + pad chord.
        let notes = [55.0, 55.0, 65.4, 49.0];
        let bf = notes[((self.t / (beat * 4.0)) as usize) % 4];
        self.bass_phase = (self.bass_phase + bf / SR).fract();
        let bass = (self.bass_phase * 2.0 - 1.0) * 0.16 * (1.0 - (-tb * 20.0).exp());
        let tt = self.t as f32;
        let pad = [220.0f32, 277.2, 329.6, 440.0]
            .iter()
            .map(|f| (tt * f * std::f32::consts::TAU).sin())
            .sum::<f32>()
            * 0.03
            * (0.6 + 0.4 * (tt * 0.5).sin());
        self.t += dt;
        (kick + hat + snare + bass + pad) * 0.7
    }
}

fn fake_album_art() -> Vec<u8> {
    let n = crate::media::ART_SIZE as usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let fx = x as f32 / n as f32;
            let fy = y as f32 / n as f32;
            let d = ((fx - 0.5).powi(2) + (fy - 0.6).powi(2)).sqrt();
            let sun = (1.0 - d / 0.25).clamp(0.0, 1.0);
            let r = (40.0 + 200.0 * fy + 120.0 * sun).min(255.0);
            let g = (20.0 + 60.0 * fx + 150.0 * sun).min(255.0);
            let b = (120.0 - 80.0 * fy + 40.0 * (1.0 - fx)).clamp(0.0, 255.0);
            let i = (y * n + x) * 4;
            px[i..i + 4].copy_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    px
}

/// Pure-audio check: feed 8 s of the synth through the analyzer and dump the onset curve.
fn audio_selftest(dir: &Path) -> String {
    let mut synth = Synth { t: 0.0, kick_phase: 0.0, bass_phase: 0.0, rng: 0x1234_5678, hp: 0.0 };
    let mut off = Offline::new(SR, crate::audio_params(&Settings::default()));
    off.enable_trace();
    let mut buf = vec![0.0f32; 480];
    let mut last = 0u32;
    let mut phases = Vec::new();
    let beat_len = 60.0 / BPM as f64;
    for _ in 0..(8.0 * SR / 480.0) as usize {
        for s in buf.iter_mut() {
            *s = synth.sample();
        }
        let f = off.push(&buf);
        if f.beat_count != last {
            last = f.beat_count;
            phases.push((((synth.t % beat_len) / beat_len) * 100.0).round() / 100.0);
        }
    }
    let trace = off.take_trace();
    let mut csv = String::from("t,onset,thresh\n");
    for (t, o, th) in &trace {
        csv += &format!("{t:.4},{o:.5},{th:.5}\n");
    }
    let _ = std::fs::write(dir.join("onset_trace.csv"), csv);
    format!("audio-only: {} beats in 8 s (expected {}), phases {:?}\n", phases.len(), (8.0 * BPM / 60.0) as u32, phases)
}

pub fn run(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let audio_report = audio_selftest(dir);
    println!("{audio_report}");
    let (w, h) = (1280u32, 720u32);
    let instance = crate::make_instance();
    let t0 = Instant::now();
    let mut gpu = match pollster::block_on(Gpu::new(&instance, None, w, h, 1.0)) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("GPU init failed: {e}");
            return;
        }
    };
    let init_ms = t0.elapsed().as_millis();
    let settings = Settings::default();
    let mut report = format!("adapter: {}\ninit (incl. shader compile): {init_ms} ms\n", gpu.adapter_name);

    let art = fake_album_art();
    let pal = palette::extract(&art, crate::media::ART_SIZE as usize, crate::media::ART_SIZE as usize);
    report += &format!("album palette: {:?}\n", pal);

    let mut synth = Synth { t: 0.0, kick_phase: 0.0, bass_phase: 0.0, rng: 0x1234_5678, hp: 0.0 };
    let mut off = Offline::new(SR, crate::audio_params(&settings));
    let mut vis = VisState::new(PRESETS[0].1);
    vis.pal_target = pal;

    let fps = 60.0f32;
    let spf = (SR / fps) as usize;
    let mut buf = vec![0.0f32; spf];
    let mut time = 0.0f32;
    let mut beats = 0u32;
    let mut last_bc = 0u32;
    let mut phases: Vec<f32> = Vec::new();

    let mut step = |gpu: &mut Gpu, vis: &mut VisState, capture: bool| -> Option<(u32, u32, Vec<u8>)> {
        for s in buf.iter_mut() {
            *s = synth.sample();
        }
        let f = off.push(&buf);
        if f.beat_count != last_bc {
            beats += 1;
            last_bc = f.beat_count;
            let beat_len = 60.0 / BPM as f64;
            phases.push(((synth.t % beat_len) / beat_len) as f32);
        }
        let dt = 1.0 / fps;
        time += dt;
        vis.update(&f, dt, Instant::now());
        let g = vis.globals(&settings, gpu.scene_size(), time, false);
        gpu.upload(&g, &vis.audio_buffer());
        if capture {
            Some(gpu.capture())
        } else {
            gpu.step_offscreen();
            None
        }
    };

    let save = |name: &str, img: (u32, u32, Vec<u8>)| {
        let (w, h, mut data) = img;
        if matches!(gpu_format_is_bgra(), true) {
            for px in data.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let path = dir.join(name);
        let _ = image::save_buffer(&path, &data, w, h, image::ColorType::Rgba8);
    };

    let t = Instant::now();
    for _ in 0..150 {
        step(&mut gpu, &mut vis, false);
    }
    let _ = gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    let ms = t.elapsed().as_secs_f32() * 1000.0 / 150.0;
    report += &format!("render: {ms:.2} ms/frame (CPU+GPU, 1280x720, unthrottled)
");
    // Capture right on a beat and between beats.
    for _ in 0..120 {
        let cap = step(&mut gpu, &mut vis, true);
        if vis.kick > 0.8 {
            if let Some(img) = cap {
                save("terminal_beat.png", img);
            }
            break;
        }
    }
    for _ in 0..12 {
        step(&mut gpu, &mut vis, false);
    }
    if let Some(img) = step(&mut gpu, &mut vis, true) {
        save("terminal.png", img);
    }

    let expected = (time * BPM / 60.0) as u32;
    let phases: Vec<f32> = phases.iter().map(|p| (p * 100.0).round() / 100.0).collect();
    report += &format!(
        "beats detected: {beats} over {time:.1} s (expected ~{expected})\nbpm estimate: {:.1}\nbeat phases (0 = on the kick): {phases:?}\n",
        vis.bpm
    );
    let _ = std::fs::write(dir.join("report.txt"), &report);
    println!("{report}");
}

fn gpu_format_is_bgra() -> bool {
    // Offscreen capture renders to Rgba8UnormSrgb (see Gpu::new with no surface).
    false
}
