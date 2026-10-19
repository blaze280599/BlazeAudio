//! WASAPI loopback capture + spectrum analysis.
//!
//! Analysis runs inside the capture callback at a fixed hop (256 samples ≈ 5 ms @ 48 kHz),
//! so band smoothing and beat detection are independent of the render frame rate, and when
//! nothing is playing WASAPI stops delivering packets and the analysis costs nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

pub const NUM_BANDS: usize = 64;
pub const WAVE_LEN: usize = 2048;
const FFT_SIZE: usize = 2048;
const HOP: usize = 256;
const FLUX_HIST: usize = 160;
const FLUX_LAG: usize = 4;
const ODF_LEN: usize = 1024;
const MIN_FREQ: f32 = 30.0;
const MAX_FREQ: f32 = 16_000.0;

#[derive(Clone, Copy, Debug)]
pub struct AudioParams {
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub sensitivity: f32,
    pub beat_sensitivity: f32,
}

#[derive(Clone)]
pub struct AudioFrame {
    pub bands: [f32; NUM_BANDS],
    pub wave: [f32; WAVE_LEN],
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub energy: f32,
    /// Beat envelope: 1.0 on a detected beat, decays exponentially.
    pub beat: f32,
    pub beat_strength: f32,
    pub beat_count: u32,
    /// High-frequency onset envelope (hi-hats / snares).
    pub hat: f32,
    pub flux: f32,
    pub bpm: f32,
    pub drop_count: u32,
    pub level: f32,
    pub updated: Option<Instant>,
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self {
            bands: [0.0; NUM_BANDS],
            wave: [0.0; WAVE_LEN],
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            energy: 0.0,
            beat: 0.0,
            beat_strength: 0.0,
            beat_count: 0,
            hat: 0.0,
            flux: 0.0,
            bpm: 0.0,
            drop_count: 0,
            level: 0.0,
            updated: None,
        }
    }
}

struct Shared {
    frame: Mutex<AudioFrame>,
    params: Mutex<AudioParams>,
    device: Mutex<String>,
    stop: AtomicBool,
}

pub struct AudioEngine {
    shared: Arc<Shared>,
}

impl AudioEngine {
    pub fn start(params: AudioParams) -> Self {
        let shared = Arc::new(Shared {
            frame: Mutex::new(AudioFrame::default()),
            params: Mutex::new(params),
            device: Mutex::new(String::from("(no device)")),
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("audio-capture".into())
            .stack_size(256 * 1024)
            .spawn(move || capture_loop(s))
            .expect("spawn audio thread");
        Self { shared }
    }

    pub fn snapshot(&self) -> AudioFrame {
        self.shared.frame.lock().unwrap().clone()
    }

    pub fn set_params(&self, p: AudioParams) {
        *self.shared.params.lock().unwrap() = p;
    }

    pub fn device_name(&self) -> String {
        self.shared.device.lock().unwrap().clone()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn capture_loop(shared: Arc<Shared>) {
    let host = cpal::default_host();
    while !shared.stop.load(Ordering::Relaxed) {
        let Some(device) = host.default_output_device() else {
            *shared.device.lock().unwrap() = "(no output device)".into();
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let dev_id = device.id().ok();
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "Default output".into());

        let config = match device.default_output_config() {
            Ok(c) => c,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let sample_rate = config.sample_rate() as f32;
        let channels = config.channels() as usize;
        let format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.config();

        let failed = Arc::new(AtomicBool::new(false));
        let err_flag = failed.clone();
        let err_cb = move |e: cpal::Error| {
            // Loopback routinely flags data discontinuities (reported as Xrun) whenever the
            // audio engine idles or a stream starts; those are harmless, so only rebuild on
            // errors that actually invalidate the stream.
            if !matches!(e.kind(), cpal::ErrorKind::Xrun) {
                #[cfg(debug_assertions)]
                eprintln!("[audio] stream error: {e}");
                err_flag.store(true, Ordering::Relaxed);
            }
        };

        let mut analyzer = Analyzer::new(sample_rate, channels, shared.clone());
        if std::env::var_os("BLAZE_TRACE").is_some() {
            analyzer.trace = Some(Vec::new());
        }
        // Building an *input* stream on an *output* device makes cpal's WASAPI host use loopback.
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                stream_config,
                move |data: &[f32], _| analyzer.push(data),
                err_cb,
                None,
            ),
            cpal::SampleFormat::I16 => {
                let mut tmp = Vec::new();
                device.build_input_stream::<i16, _, _>(
                    stream_config,
                    move |data: &[i16], _| {
                        tmp.clear();
                        tmp.extend(data.iter().map(|&s| s as f32 / 32768.0));
                        analyzer.push(&tmp);
                    },
                    err_cb,
                    None,
                )
            }
            cpal::SampleFormat::I32 => {
                let mut tmp = Vec::new();
                device.build_input_stream::<i32, _, _>(
                    stream_config,
                    move |data: &[i32], _| {
                        tmp.clear();
                        tmp.extend(data.iter().map(|&s| s as f32 / 2_147_483_648.0));
                        analyzer.push(&tmp);
                    },
                    err_cb,
                    None,
                )
            }
            _ => {
                *shared.device.lock().unwrap() = format!("{name} (unsupported format)");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        let stream = match stream {
            Ok(s) => s,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        if stream.play().is_err() {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        *shared.device.lock().unwrap() = name;

        // Supervise: rebuild on stream error or when the default output device changes.
        let mut ticks = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if shared.stop.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
                #[cfg(debug_assertions)]
                eprintln!("[audio] rebuild: stream error={}", failed.load(Ordering::Relaxed));
                break;
            }
            ticks += 1;
            if ticks % 4 == 0 {
                let current = host.default_output_device().and_then(|d| d.id().ok());
                if current != dev_id {
                    #[cfg(debug_assertions)]
                    eprintln!("[audio] rebuild: default device changed {:?} -> {:?}", dev_id, current);
                    break;
                }
            }
        }
        drop(stream);
    }
}

struct Analyzer {
    shared: Arc<Shared>,
    channels: usize,
    hop_dt: f32,
    ring: Vec<f32>,
    pos: usize,
    pending: usize,
    time: f64,

    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    norm: f32,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    mag: Vec<f32>,
    lv_hist: [[f32; NUM_BANDS]; FLUX_LAG],
    lv_pos: usize,

    band_bins: Vec<(f32, f32)>,
    band_tilt: Vec<f32>,
    band_class: Vec<u8>,
    bands: [f32; NUM_BANDS],
    agc_db: f32,

    band_fc: Vec<f32>,
    flux_hist: [f32; FLUX_HIST],
    hat_hist: [f32; FLUX_HIST],
    hist_pos: usize,
    above: bool,
    last_beat: f64,
    last_hat: f64,
    beat: f32,
    beat_strength: f32,
    beat_count: u32,
    hat: f32,
    odf: Vec<f32>,
    odf_pos: usize,
    since_tempo: u32,
    acf: Vec<f32>,
    bpm_candidate: f32,
    bpm: f32,

    energy_short: f32,
    energy_long: f32,
    last_drop: f64,
    drop_count: u32,
    rms_long: f32,
    pub trace: Option<Vec<(f32, f32, f32)>>,
    flux_smooth: f32,
}

impl Analyzer {
    fn new(sample_rate: f32, channels: usize, shared: Arc<Shared>) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let input = fft.make_input_vec();
        let spectrum = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();

        // Blackman-Harris window: low sidelobes keep bass bands from bleeding upward.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = 2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32;
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
            })
            .collect();
        let norm = 2.0 / window.iter().sum::<f32>();

        let bin_hz = sample_rate / FFT_SIZE as f32;
        let nyq = sample_rate * 0.5;
        let max_f = MAX_FREQ.min(nyq * 0.95);
        let mut band_bins = Vec::with_capacity(NUM_BANDS);
        let mut band_tilt = Vec::with_capacity(NUM_BANDS);
        let mut band_class = Vec::with_capacity(NUM_BANDS);
        let mut band_fc = Vec::with_capacity(NUM_BANDS);
        for b in 0..NUM_BANDS {
            let f0 = MIN_FREQ * (max_f / MIN_FREQ).powf(b as f32 / NUM_BANDS as f32);
            let f1 = MIN_FREQ * (max_f / MIN_FREQ).powf((b + 1) as f32 / NUM_BANDS as f32);
            band_bins.push((f0 / bin_hz, f1 / bin_hz));
            let fc = (f0 * f1).sqrt();
            // Pink-noise compensation (+3 dB/oct relative to 1 kHz) so highs aren't dwarfed.
            band_fc.push(fc);
            band_tilt.push(3.0 * (fc / 1000.0).log2());
            band_class.push(if fc < 250.0 {
                0
            } else if fc < 2500.0 {
                1
            } else {
                2
            });
        }

        Self {
            shared,
            channels: channels.max(1),
            hop_dt: HOP as f32 / sample_rate,
            ring: vec![0.0; FFT_SIZE],
            pos: 0,
            pending: 0,
            time: 0.0,
            fft,
            window,
            norm,
            input,
            spectrum,
            scratch,
            mag: vec![0.0; FFT_SIZE / 2 + 1],
            lv_hist: [[0.0; NUM_BANDS]; FLUX_LAG],
            lv_pos: 0,
            band_bins,
            band_tilt,
            band_class,
            bands: [0.0; NUM_BANDS],
            agc_db: -30.0,
            band_fc,
            flux_hist: [0.0; FLUX_HIST],
            hat_hist: [0.0; FLUX_HIST],
            hist_pos: 0,
            above: false,
            last_beat: -10.0,
            last_hat: -10.0,
            beat: 0.0,
            beat_strength: 0.0,
            beat_count: 0,
            hat: 0.0,
            odf: vec![0.0; ODF_LEN],
            odf_pos: 0,
            since_tempo: 0,
            acf: Vec::new(),
            bpm_candidate: 0.0,
            bpm: 0.0,
            energy_short: 0.0,
            energy_long: 0.0,
            last_drop: -100.0,
            drop_count: 0,
            rms_long: 0.0,
            trace: None,
            flux_smooth: 0.0,
        }
    }

    fn push(&mut self, data: &[f32]) {
        let ch = self.channels;
        let inv = 1.0 / ch as f32;
        for frame in data.chunks_exact(ch) {
            let mono = frame.iter().sum::<f32>() * inv;
            self.ring[self.pos] = mono;
            self.pos = (self.pos + 1) % FFT_SIZE;
            self.pending += 1;
            if self.pending >= HOP {
                self.pending = 0;
                self.process();
            }
        }
    }

    /// Pick the tempo whose lag maximizes the onset autocorrelation, weighted by a
    /// log-normal prior around 120 BPM (resolves half/double-tempo ambiguity).
    fn update_tempo(&mut self) {
        let n = ODF_LEN;
        let mean = self.odf.iter().sum::<f32>() / n as f32;
        let mut x = [0.0f32; ODF_LEN];
        for (i, v) in x.iter_mut().enumerate() {
            *v = self.odf[(self.odf_pos + i) % n] - mean;
        }
        let e0: f32 = x.iter().map(|v| v * v).sum();
        if e0 < 1e-7 {
            return;
        }
        let lag_min = ((60.0 / 185.0) / self.hop_dt).floor().max(2.0) as usize;
        let lag_max = (((60.0 / 70.0) / self.hop_dt).ceil() as usize).min(n / 2);
        self.acf.resize(lag_max + 2, 0.0);
        for lag in (lag_min - 1)..=(lag_max + 1) {
            let mut s = 0.0f32;
            for i in lag..n {
                s += x[i] * x[i - lag];
            }
            self.acf[lag] = s / e0;
        }
        let (mut best_lag, mut best_score) = (0usize, 0.0f32);
        for lag in lag_min..=lag_max {
            let bpm = 60.0 / (lag as f32 * self.hop_dt);
            let o = (bpm / 120.0).log2() / 0.7;
            let score = self.acf[lag] * (-0.5 * o * o).exp();
            if score > best_score {
                best_score = score;
                best_lag = lag;
            }
        }
        if best_lag == 0 || self.acf[best_lag] < 0.06 {
            return;
        }
        // Parabolic interpolation for sub-hop precision.
        let (a, b, c) = (self.acf[best_lag - 1], self.acf[best_lag], self.acf[best_lag + 1]);
        let denom = a - 2.0 * b + c;
        let off = if denom.abs() > 1e-6 { (0.5 * (a - c) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        let new_bpm = 60.0 / ((best_lag as f32 + off) * self.hop_dt);
        if self.bpm > 0.0 && (new_bpm / self.bpm - 1.0).abs() < 0.04 {
            self.bpm += (new_bpm - self.bpm) * 0.3;
        } else if self.bpm_candidate > 0.0 && (new_bpm / self.bpm_candidate - 1.0).abs() < 0.04 {
            // Same new tempo twice in a row: accept the change.
            self.bpm = new_bpm;
            self.bpm_candidate = 0.0;
        } else {
            self.bpm_candidate = new_bpm;
        }
    }

    fn mag_at(&self, bin: f32) -> f32 {
        let i = bin.floor().max(0.0) as usize;
        let f = bin - i as f32;
        let a = self.mag[i.min(self.mag.len() - 1)];
        let b = self.mag[(i + 1).min(self.mag.len() - 1)];
        a + (b - a) * f
    }

    fn process(&mut self) {
        let dt = self.hop_dt;
        self.time += dt as f64;
        let p = *self.shared.params.lock().unwrap();

        // Unroll ring (oldest -> newest), window, FFT.
        let mut sum_sq = 0.0f32;
        for i in 0..FFT_SIZE {
            let s = self.ring[(self.pos + i) % FFT_SIZE];
            if i >= FFT_SIZE - HOP * 2 {
                sum_sq += s * s;
            }
            self.input[i] = s * self.window[i];
        }
        let rms = (sum_sq / (HOP * 2) as f32).sqrt();
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return;
        }
        for (m, c) in self.mag.iter_mut().zip(self.spectrum.iter()) {
            *m = c.norm() * self.norm;
        }

        // ---- Log-spaced bands with AGC ----
        let mut db = [0.0f32; NUM_BANDS];
        let mut max_db = -200.0f32;
        for b in 0..NUM_BANDS {
            let (lo, hi) = self.band_bins[b];
            let m = if hi - lo < 1.0 {
                self.mag_at((lo + hi) * 0.5)
            } else {
                let mut mx = 0.0f32;
                let end = (hi.floor() as usize).min(self.mag.len() - 1);
                for k in lo.ceil() as usize..=end {
                    mx = mx.max(self.mag[k]);
                }
                mx
            };
            let d = 20.0 * (m + 1e-9).log10() + self.band_tilt[b];
            db[b] = d;
            max_db = max_db.max(d);
        }
        // Fast-up / slow-down automatic gain so quiet and loud tracks both fill the screen.
        if max_db > self.agc_db {
            self.agc_db += (max_db - self.agc_db) * (1.0 - (-dt / 0.05).exp());
        } else {
            self.agc_db -= 4.0 * dt;
        }
        self.agc_db = self.agc_db.max(-55.0);
        let range = 48.0;
        let top = self.agc_db + 2.0 - (p.sensitivity - 1.0) * 14.0;
        let gate = rms > 1.5e-5;

        let ka = 1.0 - (-dt / (p.attack_ms.max(1.0) * 0.001)).exp();
        let kd = 1.0 - (-dt / (p.decay_ms.max(1.0) * 0.001)).exp();
        let mut cls_sum = [0.0f32; 3];
        let mut cls_n = [0.0f32; 3];
        for b in 0..NUM_BANDS {
            let target = if gate {
                ((db[b] - (top - range)) / range).clamp(0.0, 1.0).powf(1.5)
            } else {
                0.0
            };
            let cur = self.bands[b];
            let k = if target > cur { ka } else { kd };
            self.bands[b] = cur + (target - cur) * k;
            let c = self.band_class[b] as usize;
            cls_sum[c] += self.bands[b];
            cls_n[c] += 1.0;
        }
        let bass = cls_sum[0] / cls_n[0].max(1.0);
        let mid = cls_sum[1] / cls_n[1].max(1.0);
        let treble = cls_sum[2] / cls_n[2].max(1.0);
        let energy = self.bands.iter().sum::<f32>() / NUM_BANDS as f32;

        // ---- Spectral flux ----
        // Half-wave rectified rise of the AGC-normalized log bands versus FLUX_LAG hops ago.
        // Band-level (not bin-level) magnitudes and a ~16 ms lag reject the hop-to-hop noise
        // that heavily overlapped frames produce on noisy/beating content.
        let mut lv = [0.0f32; NUM_BANDS];
        for b in 0..NUM_BANDS {
            lv[b] = ((db[b] - (top - range)) / range).clamp(0.0, 1.0);
        }
        // SuperFlux-style reference: max over the lag window, so slow wobble does not count as a rise.
        let mut reference = [0.0f32; NUM_BANDS];
        for h in &self.lv_hist {
            for b in 0..NUM_BANDS {
                reference[b] = reference[b].max(h[b]);
            }
        }
        let (mut kick, mut full, mut hi) = (0.0f32, 0.0f32, 0.0f32);
        let (mut nk, mut nh) = (0.0f32, 0.0f32);
        for b in 0..NUM_BANDS {
            let r = (lv[b] - reference[b]).max(0.0);
            let d = r * r * 4.0;
            full += d;
            if self.band_fc[b] < 150.0 {
                kick += d;
                nk += 1.0;
            } else if self.band_fc[b] > 5000.0 {
                hi += d;
                nh += 1.0;
            }
        }
        self.lv_hist[self.lv_pos] = lv;
        self.lv_pos = (self.lv_pos + 1) % FLUX_LAG;
        kick /= nk.max(1.0);
        full /= NUM_BANDS as f32;
        hi /= nh.max(1.0);
        let onset = if gate { kick + full * 0.3 } else { 0.0 };
        let hi = if gate { hi } else { 0.0 };

        let (mean, std) = mean_std(&self.flux_hist);
        let (hmean, hstd) = mean_std(&self.hat_hist);
        self.flux_hist[self.hist_pos] = onset;
        self.hat_hist[self.hist_pos] = hi;
        self.hist_pos = (self.hist_pos + 1) % FLUX_HIST;

        // Tempo tracking: autocorrelate ~5.5 s of the onset envelope every ~0.5 s.
        self.odf[self.odf_pos] = onset;
        self.odf_pos = (self.odf_pos + 1) % ODF_LEN;
        self.since_tempo += 1;
        if self.since_tempo >= 96 {
            self.since_tempo = 0;
            self.update_tempo();
        }

        let k_sigma = 0.8 + 1.6 / p.beat_sensitivity.max(0.1);
        let thresh = mean + std * k_sigma;
        let is_above = onset > thresh && onset > mean * 1.5 && onset > 0.006;
        if let Some(t) = &mut self.trace {
            t.push((self.time as f32, onset, thresh));
            // Live capture debugging: BLAZE_TRACE=<file.csv> dumps ~10 s of the onset curve.
            if t.len() == 1900 {
                if let Some(path) = std::env::var_os("BLAZE_TRACE") {
                    let mut csv = String::from("t,onset,thresh\n");
                    for (a, b, c) in t.iter() {
                        csv += &format!("{a:.4},{b:.5},{c:.5}\n");
                    }
                    let _ = std::fs::write(path, csv);
                }
            }
        }
        // Once the tempo is known, ignore onsets closer than ~60% of a beat so pulses lock to
        // the beat instead of every off-beat hat or vocal transient.
        let refractory = if self.bpm > 0.0 { (36.0 / self.bpm as f64).max(0.17) } else { 0.17 };
        if is_above && !self.above && self.time - self.last_beat > refractory {
            self.last_beat = self.time;
            self.beat = 1.0;
            self.beat_strength = ((onset - mean) / (std * 4.0 + 1e-4)).clamp(0.25, 1.0);
            self.beat_count = self.beat_count.wrapping_add(1);
        }
        self.above = is_above;
        self.beat *= (-dt / 0.16).exp();

        if hi > hmean + hstd * 2.2 && hi > 0.002 && self.time - self.last_hat > 0.07 {
            self.last_hat = self.time;
            self.hat = 1.0;
        }
        self.hat *= (-dt / 0.09).exp();

        // ---- Drop detection: short-term energy jumping well above the long-term average ----
        self.energy_short += (energy - self.energy_short) * (1.0 - (-dt / 0.35).exp());
        self.energy_long += (energy - self.energy_long) * (1.0 - (-dt / 8.0).exp());
        if self.energy_short > self.energy_long * 1.45
            && self.energy_short > 0.32
            && self.time - self.last_drop > 12.0
        {
            self.last_drop = self.time;
            self.drop_count = self.drop_count.wrapping_add(1);
        }

        self.rms_long += (rms - self.rms_long) * (1.0 - (-dt / 1.5).exp());
        self.flux_smooth += (onset - self.flux_smooth) * (1.0 - (-dt / 0.05).exp());

        // ---- Publish ----
        let gain = (0.3 / self.rms_long.max(1e-3)).min(40.0);
        let mut f = self.shared.frame.lock().unwrap();
        f.bands = self.bands;
        // Oscilloscope window with its DC/sub-bass offset removed so the ring stays centered.
        let start = (self.pos + FFT_SIZE - WAVE_LEN) % FFT_SIZE;
        let mean = (0..WAVE_LEN).map(|i| self.ring[(start + i) % FFT_SIZE]).sum::<f32>() / WAVE_LEN as f32;
        for i in 0..WAVE_LEN {
            // Loose clamp only; the scope soft-clips, so peaks round off instead of flattening.
            f.wave[i] = ((self.ring[(start + i) % FFT_SIZE] - mean) * gain).clamp(-4.0, 4.0);
        }
        f.bass = bass;
        f.mid = mid;
        f.treble = treble;
        f.energy = energy;
        f.beat = self.beat;
        f.beat_strength = self.beat_strength;
        f.beat_count = self.beat_count;
        f.hat = self.hat;
        f.flux = (self.flux_smooth / (mean + std * 3.0 + 1e-3)).clamp(0.0, 1.0);
        f.bpm = self.bpm;
        f.drop_count = self.drop_count;
        f.level = rms;
        f.updated = Some(Instant::now());
    }
}

fn mean_std(v: &[f32]) -> (f32, f32) {
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n;
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n;
    (mean, var.sqrt())
}

/// Runs the exact same analysis offline on a mono buffer (used by `--snapshot` / self-test).
pub struct Offline {
    an: Analyzer,
}

impl Offline {
    pub fn new(sample_rate: f32, params: AudioParams) -> Self {
        let shared = Arc::new(Shared {
            frame: Mutex::new(AudioFrame::default()),
            params: Mutex::new(params),
            device: Mutex::new(String::new()),
            stop: AtomicBool::new(false),
        });
        Self { an: Analyzer::new(sample_rate, 1, shared) }
    }

    pub fn push(&mut self, mono: &[f32]) -> AudioFrame {
        self.an.push(mono);
        self.an.shared.frame.lock().unwrap().clone()
    }
}

impl Offline {
    pub fn enable_trace(&mut self) {
        self.an.trace = Some(Vec::new());
    }
    pub fn take_trace(&mut self) -> Vec<(f32, f32, f32)> {
        self.an.trace.take().unwrap_or_default()
    }
}
