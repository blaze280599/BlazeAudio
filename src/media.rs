//! Now-playing metadata + album art via Windows' Global System Media Transport Controls.
//! Prefers a Spotify session when several apps are publishing media sessions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProps,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::palette::{self, Palette};

pub const ART_SIZE: u32 = 256;

#[derive(Clone, Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: String,
    pub playing: bool,
    /// ART_SIZE x ART_SIZE sRGB RGBA8.
    pub art: Option<Arc<Vec<u8>>>,
    pub palette: Option<Palette>,
    /// Bumps whenever title/artist changes.
    pub track_version: u64,
    /// Bumps whenever art/palette changes.
    pub art_version: u64,
}

struct Shared {
    info: Mutex<TrackInfo>,
    stop: AtomicBool,
}

pub struct MediaWatcher {
    shared: Arc<Shared>,
}

impl MediaWatcher {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            info: Mutex::new(TrackInfo::default()),
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("media-session".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                unsafe {
                    let _ = RoInitialize(RO_INIT_MULTITHREADED);
                }
                watch_loop(&s);
            })
            .expect("spawn media thread");
        Self { shared }
    }

    pub fn snapshot(&self) -> TrackInfo {
        self.shared.info.lock().unwrap().clone()
    }

    pub fn versions(&self) -> (u64, u64) {
        let i = self.shared.info.lock().unwrap();
        (i.track_version, i.art_version)
    }
}

impl Drop for MediaWatcher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn pick_session(mgr: &SessionManager) -> Option<Session> {
    if let Ok(sessions) = mgr.GetSessions() {
        let n = sessions.Size().unwrap_or(0);
        for i in 0..n {
            if let Ok(s) = sessions.GetAt(i) {
                let id = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
                if id.to_ascii_lowercase().contains("spotify") {
                    return Some(s);
                }
            }
        }
    }
    mgr.GetCurrentSession().ok()
}

/// Raw encoded thumbnail bytes (JPEG/PNG), undecoded.
fn read_thumb(props: &MediaProps) -> Option<Vec<u8>> {
    let thumb = props.Thumbnail().ok()?;
    let stream = thumb.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 16 * 1024 * 1024 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

fn decode_art(buf: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(buf).ok()?.to_rgba8();
    // Spotify thumbnails often carry a letterbox; crop to the centered square first.
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let cropped = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let resized = image::imageops::resize(&cropped, ART_SIZE, ART_SIZE, image::imageops::FilterType::Triangle);
    Some(resized.into_raw())
}

fn hs(r: windows::core::Result<HSTRING>) -> String {
    r.map(|h| h.to_string()).unwrap_or_default()
}

fn watch_loop(shared: &Shared) {
    let mut mgr: Option<SessionManager> = None;
    let mut last_key = String::new();
    // Art can lag the title change by a moment, so re-fetch a few times after a change.
    let mut art_retry_until: Option<Instant> = None;
    let mut last_art_hash: u64 = 0;

    while !shared.stop.load(Ordering::Relaxed) {
        if mgr.is_none() {
            mgr = SessionManager::RequestAsync().ok().and_then(|op| op.join().ok());
            if mgr.is_none() {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        }
        let m = mgr.as_ref().unwrap();

        match pick_session(m) {
            Some(session) => {
                let props = session.TryGetMediaPropertiesAsync().ok().and_then(|op| op.join().ok());
                let playing = session
                    .GetPlaybackInfo()
                    .and_then(|i| i.PlaybackStatus())
                    .map(|s| s == PlaybackStatus::Playing)
                    .unwrap_or(false);
                let source = hs(session.SourceAppUserModelId());
                if let Some(props) = props {
                    let title = hs(props.Title());
                    let artist = hs(props.Artist());
                    let album = hs(props.AlbumTitle());
                    let key = format!("{title}\u{1}{artist}");
                    let changed = key != last_key;
                    if changed {
                        last_key = key;
                        art_retry_until = Some(Instant::now() + Duration::from_secs(4));
                        let mut i = shared.info.lock().unwrap();
                        i.title = title;
                        i.artist = artist;
                        i.album = album;
                        i.track_version += 1;
                    }
                    if art_retry_until.is_some_and(|t| Instant::now() < t) {
                        // Hash the encoded bytes so re-fetches of the same cover skip the decode.
                        let fresh = read_thumb(&props).map(|buf| (hash(&buf), buf)).filter(|(h, _)| *h != last_art_hash);
                        if let Some((h, buf)) = fresh {
                            last_art_hash = h;
                            if let Some(art) = decode_art(&buf) {
                                let pal = palette::extract(&art, ART_SIZE as usize, ART_SIZE as usize);
                                let mut i = shared.info.lock().unwrap();
                                i.art = Some(Arc::new(art));
                                i.palette = Some(pal);
                                i.art_version += 1;
                            }
                        }
                    } else {
                        art_retry_until = None;
                    }
                }
                let mut i = shared.info.lock().unwrap();
                i.playing = playing;
                i.source = source;
            }
            None => {
                if !last_key.is_empty() {
                    last_key.clear();
                    last_art_hash = 0;
                    let mut i = shared.info.lock().unwrap();
                    let (tv, av) = (i.track_version + 1, i.art_version + 1);
                    *i = TrackInfo::default();
                    i.track_version = tv;
                    i.art_version = av;
                }
            }
        }
        let wait = if art_retry_until.is_some() { 700 } else { 1000 };
        std::thread::sleep(Duration::from_millis(wait));
    }
}

fn hash(data: &[u8]) -> u64 {
    // FNV-1a over a sparse sample; enough to detect a different cover.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.iter().step_by(7) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
