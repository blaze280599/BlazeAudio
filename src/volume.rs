//! System output volume: the default render device's master level and mute, through Core Audio
//! (`IAudioEndpointVolume`). A worker thread owns the COM objects; the UI reads atomics and
//! sends changes over a channel, so it never blocks on COM.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

/// How often the worker re-reads the level (other apps and the volume keys change it too).
const POLL: Duration = Duration::from_millis(250);
/// How often it re-resolves the default device, so switching outputs is picked up.
const REFRESH: Duration = Duration::from_secs(2);

enum Cmd {
    Level(f32),
    Mute(bool),
}

#[derive(Default)]
struct Shared {
    /// f32 bits, 0..=1.
    level: AtomicU32,
    muted: AtomicBool,
    available: AtomicBool,
    /// Bumped by every UI change, so a poll that started before it doesn't overwrite it.
    generation: AtomicU32,
}

pub struct Volume {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
}

impl Volume {
    pub fn new() -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = channel::<Cmd>();
        let sh = shared.clone();
        let _ = std::thread::Builder::new().name("volume".into()).spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
                return;
            };
            let open = || -> Option<IAudioEndpointVolume> {
                let dev = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
            };
            let mut ep = open();
            let mut opened = Instant::now();
            loop {
                match rx.recv_timeout(POLL) {
                    Ok(cmd) => {
                        if let Some(v) = &ep {
                            let _ = match cmd {
                                Cmd::Level(l) => v.SetMasterVolumeLevelScalar(l, std::ptr::null()),
                                Cmd::Mute(m) => v.SetMute(m, std::ptr::null()),
                            };
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if ep.is_none() || opened.elapsed() > REFRESH {
                    ep = open();
                    opened = Instant::now();
                }
                let gen = sh.generation.load(Ordering::Acquire);
                let read = ep.as_ref().and_then(|v| Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool())));
                match read {
                    Some((level, muted)) => {
                        if sh.generation.load(Ordering::Acquire) == gen {
                            sh.level.store(level.clamp(0.0, 1.0).to_bits(), Ordering::Release);
                            sh.muted.store(muted, Ordering::Release);
                        }
                        sh.available.store(true, Ordering::Release);
                    }
                    None => {
                        ep = None;
                        sh.available.store(false, Ordering::Release);
                    }
                }
            }
        });
        Self { shared, tx }
    }

    /// False until the first read succeeds, or while there is no output device.
    pub fn available(&self) -> bool {
        self.shared.available.load(Ordering::Acquire)
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Acquire))
    }

    pub fn muted(&self) -> bool {
        self.shared.muted.load(Ordering::Acquire)
    }

    pub fn set_level(&self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.level.store(level.to_bits(), Ordering::Release);
        let _ = self.tx.send(Cmd::Level(level));
        // Raising the volume while muted would otherwise do nothing audible.
        if self.muted() && level > 0.0 {
            self.set_muted(false);
        }
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.muted.store(muted, Ordering::Release);
        let _ = self.tx.send(Cmd::Mute(muted));
    }

    /// Step by `delta` (0..1 scale), snapped to whole percent.
    pub fn nudge(&self, delta: f32) {
        self.set_level(((self.level() + delta) * 100.0).round() / 100.0);
    }
}
