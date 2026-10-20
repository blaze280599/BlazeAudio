//! Now-playing metadata + album art via Windows' Global System Media Transport Controls.
//! Prefers a Spotify session when several apps are publishing media sessions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// A transport command for the media session on screen.
#[derive(Clone, Copy)]
pub enum MediaCmd {
    PlayPause,
    Next,
    Previous,
    /// Jump to this many seconds into the track.
    Seek(f32),
}

/// Play state and timeline. Changes every second or so, so it's read every frame instead of
/// riding on the track/art versions.
#[derive(Clone, Copy, Default)]
pub struct Playback {
    pub playing: bool,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    /// Seconds into the track when `at` was taken.
    pub position: f32,
    /// Track length in seconds; 0 when the app doesn't report a timeline.
    pub duration: f32,
    pub at: Option<Instant>,
}

impl Playback {
    /// Position now, carried forward from the last reading while playing.
    pub fn position_now(&self) -> f32 {
        let run = if self.playing { self.at.map_or(0.0, |t| t.elapsed().as_secs_f32()) } else { 0.0 };
        (self.position + run).clamp(0.0, self.duration.max(0.0))
    }
}

#[derive(Clone, Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: String,
    pub playback: Playback,
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
    /// Where the last seek went and when. The app takes a moment to report its new position,
    /// so until it does the timeline shows this instead of snapping back.
    seek_hold: Mutex<Option<(f32, Instant)>>,
    stop: AtomicBool,
}

/// How long a seek is trusted over what the app reports.
const SEEK_HOLD: Duration = Duration::from_secs(3);

pub struct MediaWatcher {
    shared: Arc<Shared>,
    cmds: Sender<MediaCmd>,
}

impl MediaWatcher {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            info: Mutex::new(TrackInfo::default()),
            seek_hold: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        let (cmds, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("media-session".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                unsafe {
                    let _ = RoInitialize(RO_INIT_MULTITHREADED);
                }
                watch_loop(&s, &rx);
            })
            .expect("spawn media thread");
        Self { shared, cmds }
    }

    /// Asks the media session to play/pause or skip. Handled on the media thread.
    pub fn send(&self, cmd: MediaCmd) {
        {
            let mut hold = self.shared.seek_hold.lock().unwrap();
            match cmd {
                MediaCmd::Seek(pos) => *hold = Some((pos, Instant::now())),
                // A new track starts from its own beginning.
                MediaCmd::Next | MediaCmd::Previous => *hold = None,
                MediaCmd::PlayPause => {}
            }
        }
        let _ = self.cmds.send(cmd);
    }

    pub fn playback(&self) -> Playback {
        let mut pb = self.shared.info.lock().unwrap().playback;
        if let Some((pos, at)) = *self.shared.seek_hold.lock().unwrap() {
            let age = at.elapsed();
            let expect = pos + if pb.playing { age.as_secs_f32() } else { 0.0 };
            // Once the app's own position agrees, it takes over again.
            if age < SEEK_HOLD && (pb.position_now() - expect).abs() > 1.0 {
                pb.position = expect.clamp(0.0, pb.duration);
                pb.at = Some(Instant::now());
            }
        }
        pb
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

/// Play state, available controls and timeline of a session.
fn read_playback(session: &Session) -> Playback {
    let mut pb = Playback { at: Some(Instant::now()), ..Playback::default() };
    if let Ok(info) = session.GetPlaybackInfo() {
        pb.playing = info.PlaybackStatus().is_ok_and(|s| s == PlaybackStatus::Playing);
        if let Ok(c) = info.Controls() {
            pb.can_play_pause = c.IsPlayPauseToggleEnabled().unwrap_or(false);
            pb.can_next = c.IsNextEnabled().unwrap_or(false);
            pb.can_previous = c.IsPreviousEnabled().unwrap_or(false);
            pb.can_seek = c.IsPlaybackPositionEnabled().unwrap_or(false);
        }
    }
    if let Ok(tl) = session.GetTimelineProperties() {
        let secs = |d: windows::core::Result<windows::Foundation::TimeSpan>| d.map_or(0.0, |d| d.Duration as f64 * 1e-7);
        let (start, end) = (secs(tl.StartTime()), secs(tl.EndTime()));
        let mut pos = secs(tl.Position()) - start;
        // The app reports the position as of its last update (Spotify only on play/pause and
        // seeks), so carry it forward to now while playing.
        if pb.playing {
            if let (Ok(updated), Ok(now)) = (tl.LastUpdatedTime(), SystemTime::now().duration_since(UNIX_EPOCH)) {
                // DateTime counts 100 ns ticks from 1601; the Unix epoch is 11644473600 s later.
                let updated = updated.UniversalTime as f64 * 1e-7 - 11_644_473_600.0;
                let since = now.as_secs_f64() - updated;
                if updated > 0.0 && (0.0..86_400.0).contains(&since) {
                    pos += since;
                }
            }
        }
        let duration = end - start;
        if duration > 1.0 {
            pb.duration = duration as f32;
            pb.position = pos.clamp(0.0, duration) as f32;
        }
    }
    pb
}

fn run_cmd(session: &Session, cmd: MediaCmd) {
    let op = match cmd {
        MediaCmd::PlayPause => session.TryTogglePlayPauseAsync(),
        MediaCmd::Next => session.TrySkipNextAsync(),
        MediaCmd::Previous => session.TrySkipPreviousAsync(),
        MediaCmd::Seek(secs) => {
            // Positions are absolute ticks (100 ns), counted from the timeline's start.
            let start = session.GetTimelineProperties().and_then(|t| t.StartTime()).map_or(0, |d| d.Duration);
            session.TryChangePlaybackPositionAsync(start + (secs.max(0.0) as f64 * 1e7) as i64)
        }
    };
    let _ = op.and_then(|op| op.join());
}

fn watch_loop(shared: &Shared, cmds: &Receiver<MediaCmd>) {
    let mut mgr: Option<SessionManager> = None;
    let mut last_key = String::new();
    // Art can lag the title change by a moment, so re-fetch a few times after a change.
    let mut art_retry_until: Option<Instant> = None;
    let mut last_art_hash: u64 = 0;
    // Poll faster for a moment after a command, so the play button and title catch up quickly.
    let mut fast_until: Option<Instant> = None;
    let mut pending: Vec<MediaCmd> = Vec::new();

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
                if !pending.is_empty() {
                    for cmd in pending.drain(..) {
                        run_cmd(&session, cmd);
                    }
                    fast_until = Some(Instant::now() + Duration::from_secs(2));
                }
                let props = session.TryGetMediaPropertiesAsync().ok().and_then(|op| op.join().ok());
                let playback = read_playback(&session);
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
                i.playback = playback;
                i.source = source;
            }
            None => {
                // Nothing to send it to; don't let it fire when a session turns up later.
                pending.clear();
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
        let fast = fast_until.is_some_and(|t| Instant::now() < t);
        let wait = if fast { 200 } else if art_retry_until.is_some() { 700 } else { 1000 };
        match cmds.recv_timeout(Duration::from_millis(wait)) {
            Ok(cmd) => {
                // Take everything queued up. A run of seeks (a held arrow key, wheel spins) only
                // needs its last one.
                for cmd in std::iter::once(cmd).chain(cmds.try_iter()) {
                    match (pending.last_mut(), cmd) {
                        (Some(last @ MediaCmd::Seek(_)), MediaCmd::Seek(_)) => *last = cmd,
                        _ => pending.push(cmd),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
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
