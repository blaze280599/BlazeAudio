//! Update checks against this repo's GitHub releases, on an `update` worker thread. This is
//! the app's only network use, and the Updates setting can turn it off.
//!
//! The app is a portable exe, so an update replaces that exe in place. Windows won't let a
//! running exe be overwritten but does let it be renamed, so the running file moves aside to
//! `<exe>.old`, the download takes its name, and the next launch runs the new version. The
//! `.old` file is deleted on the next start.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::{Array, HSTRING};
use windows::Foundation::Uri;
use windows::Security::Cryptography::Core::{HashAlgorithmNames, HashAlgorithmProvider};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Web::Http::Filters::{HttpBaseProtocolFilter, HttpCacheReadBehavior, HttpCacheWriteBehavior};
use windows::Web::Http::{HttpClient, HttpStatusCode};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::settings::UpdateMode;

/// The version this exe was built as, from Cargo.toml. Release tags are this with a `v` in front.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST: &str = "https://api.github.com/repos/blaze280599/BlazeAudio/releases/latest";
/// The release assets every release must carry: the exe and its SHA-256 (`<hex>  blazeaudio.exe`).
const EXE_ASSET: &str = "blazeaudio.exe";
const SHA_ASSET: &str = "blazeaudio.exe.sha256";
/// How long after launch the automatic check waits, so it stays out of startup.
const LAUNCH_DELAY: Duration = Duration::from_secs(5);

/// Where the updater is, for the General section.
#[derive(Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// No release has been published yet.
    NoReleases,
    /// This newer version can be installed.
    Available(String),
    Downloading(String),
    /// The exe on disk is now this version; it runs from the next launch.
    Installed(String),
    Failed(String),
}

pub enum UpdateCmd {
    Check,
    /// Download and install the release the last check found.
    Install,
}

struct Shared {
    status: Mutex<Status>,
    /// Bumps on every status change, so the main thread copies the status only then.
    version: AtomicU64,
}

impl Shared {
    fn set(&self, s: Status) {
        if let Ok(mut cur) = self.status.lock() {
            *cur = s;
        }
        self.version.fetch_add(1, Ordering::Release);
    }
}

pub struct Updater {
    shared: Arc<Shared>,
    cmds: Sender<UpdateCmd>,
    exe: Option<PathBuf>,
}

impl Updater {
    /// Starts the worker. Unless `mode` is Off it checks a few seconds after launch, and with
    /// Auto it also installs what it finds. Dev builds skip the launch check unless
    /// `BLAZE_UPDATE_URL` is set, so running from `target/` never replaces anything unasked.
    pub fn start(mode: UpdateMode) -> Self {
        let shared = Arc::new(Shared { status: Mutex::new(Status::Idle), version: AtomicU64::new(0) });
        // Taken before any rename, so it always names the file the app was launched from.
        let exe = std::env::current_exe().ok();
        let (cmds, rx) = mpsc::channel();
        let (s, e) = (shared.clone(), exe.clone());
        let launch = if cfg!(debug_assertions) && std::env::var_os("BLAZE_UPDATE_URL").is_none() { UpdateMode::Off } else { mode };
        std::thread::Builder::new()
            .name("update".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                unsafe {
                    let _ = RoInitialize(RO_INIT_MULTITHREADED);
                }
                worker(&s, &rx, launch, e.as_deref());
            })
            .expect("spawn update thread");
        Self { shared, cmds, exe }
    }

    pub fn send(&self, cmd: UpdateCmd) {
        let _ = self.cmds.send(cmd);
    }

    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Acquire)
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Starts the exe on disk (the new version after an install). The caller exits right after;
    /// `--relaunch` makes the new copy wait for this one to close instead of focusing it.
    pub fn relaunch(&self) -> bool {
        self.exe.as_ref().is_some_and(|exe| std::process::Command::new(exe).arg("--relaunch").spawn().is_ok())
    }
}

struct Release {
    version: String,
    exe_url: Option<String>,
    sha_url: Option<String>,
}

fn worker(shared: &Shared, rx: &Receiver<UpdateCmd>, launch: UpdateMode, exe: Option<&Path>) {
    if let Some(exe) = exe {
        clean_up(exe);
    }
    // After an install, later checks compare against what's on disk, not what's running.
    let mut current = VERSION.to_owned();
    let mut found: Option<Release> = None;

    if launch != UpdateMode::Off {
        std::thread::sleep(LAUNCH_DELAY);
        found = check(shared, &current);
        if launch == UpdateMode::Auto {
            if let (Some(rel), Some(exe)) = (&found, exe) {
                if install(shared, exe, rel) {
                    current = rel.version.clone();
                    found = None;
                }
            }
        }
    }

    while let Ok(cmd) = rx.recv() {
        match cmd {
            UpdateCmd::Check => found = check(shared, &current),
            UpdateCmd::Install => match (&found, exe) {
                (Some(rel), Some(exe)) => {
                    if install(shared, exe, rel) {
                        current = rel.version.clone();
                        found = None;
                    }
                }
                _ => found = check(shared, &current),
            },
        }
    }
}

/// Deletes what an earlier update left beside the exe. Right after a restart the old copy may
/// still be closing, so this retries for a few seconds.
fn clean_up(exe: &Path) {
    let _ = std::fs::remove_file(sibling(exe, ".new"));
    let old = sibling(exe, ".old");
    for _ in 0..20 {
        if !old.exists() || std::fs::remove_file(&old).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `<exe><suffix>`, e.g. `blazeaudio.exe.old`.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut s = OsString::from(exe.as_os_str());
    s.push(suffix);
    PathBuf::from(s)
}

/// Asks GitHub for the latest release and returns it if it's newer than `current`.
fn check(shared: &Shared, current: &str) -> Option<Release> {
    shared.set(Status::Checking);
    let rel = match latest() {
        Ok(Some(r)) => r,
        Ok(None) => {
            shared.set(Status::NoReleases);
            return None;
        }
        Err(e) => {
            shared.set(Status::Failed(e));
            return None;
        }
    };
    if !is_newer(&rel.version, current) {
        shared.set(Status::UpToDate);
        return None;
    }
    if rel.exe_url.is_none() || rel.sha_url.is_none() {
        shared.set(Status::Failed(format!("Version {} is missing its download.", rel.version)));
        return None;
    }
    shared.set(Status::Available(rel.version.clone()));
    Some(rel)
}

/// Dev builds can point the updater at a local server with `BLAZE_UPDATE_URL` (a URL serving
/// JSON shaped like GitHub's `releases/latest`), to test an update without publishing one.
fn latest_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(u) = std::env::var("BLAZE_UPDATE_URL") {
        return u;
    }
    LATEST.to_owned()
}

fn latest() -> Result<Option<Release>, String> {
    let c = client().map_err(|_| "Couldn't start the network client.".to_owned())?;
    let Some(body) = get(&c, &latest_url())? else { return Ok(None) };
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| "GitHub sent something unreadable.".to_owned())?;
    let asset = |name: &str| {
        let a = v["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))?;
        a["browser_download_url"].as_str().map(str::to_owned)
    };
    let tag = v["tag_name"].as_str().unwrap_or("");
    Ok(Some(Release { version: tag.trim_start_matches(['v', 'V']).to_owned(), exe_url: asset(EXE_ASSET), sha_url: asset(SHA_ASSET) }))
}

/// Downloads, verifies and swaps in `rel`. True when the exe on disk is now the new version.
fn install(shared: &Shared, exe: &Path, rel: &Release) -> bool {
    shared.set(Status::Downloading(rel.version.clone()));
    match download_and_swap(exe, rel) {
        Ok(()) => {
            shared.set(Status::Installed(rel.version.clone()));
            true
        }
        Err(e) => {
            shared.set(Status::Failed(e));
            false
        }
    }
}

fn download_and_swap(exe: &Path, rel: &Release) -> Result<(), String> {
    let (Some(exe_url), Some(sha_url)) = (&rel.exe_url, &rel.sha_url) else { return Err("The release is missing its download.".into()) };
    let c = client().map_err(|_| "Couldn't start the network client.".to_owned())?;
    let missing = || "The download went missing; try again later.".to_owned();
    let sha = get(&c, sha_url)?.ok_or_else(missing)?;
    let bytes = get(&c, exe_url)?.ok_or_else(missing)?;

    // The checksum guards against a broken or truncated download. It comes from the same
    // release over HTTPS, so it's no defence against a compromised release, which only code
    // signing would give.
    let want = String::from_utf8_lossy(&sha).split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let got = sha256_hex(&bytes).map_err(|_| "Couldn't verify the download.".to_owned())?;
    if want.len() != 64 || got != want || !bytes.starts_with(b"MZ") {
        return Err("The download didn't match its checksum.".into());
    }

    let (new, old) = (sibling(exe, ".new"), sibling(exe, ".old"));
    let no_write = |_| "Can't write to the folder the app is in. Move it somewhere you can edit, like Documents.".to_owned();
    std::fs::write(&new, &bytes).map_err(no_write)?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(no_write(e));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        // Put the running version back so there's still an exe to launch.
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(no_write(e));
    }
    Ok(())
}

fn client() -> windows::core::Result<HttpClient> {
    let filter = HttpBaseProtocolFilter::new()?;
    let cache = filter.CacheControl()?;
    // Always ask GitHub (its API replies are cacheable for a minute), and keep the exe out of
    // the system's HTTP cache.
    cache.SetReadBehavior(HttpCacheReadBehavior::MostRecent)?;
    cache.SetWriteBehavior(HttpCacheWriteBehavior::NoCache)?;
    let c = HttpClient::Create(&filter)?;
    // GitHub's API refuses requests without a User-Agent.
    c.DefaultRequestHeaders()?.Append(&HSTRING::from("User-Agent"), &HSTRING::from(format!("BlazeAudio/{VERSION}")))?;
    Ok(c)
}

/// The body of `url`, or None on a 404. Redirects (release downloads use them) are followed.
fn get(c: &HttpClient, url: &str) -> Result<Option<Vec<u8>>, String> {
    let offline = |_| "Couldn't reach GitHub. Check your connection.".to_owned();
    let uri = Uri::CreateUri(&HSTRING::from(url)).map_err(offline)?;
    let resp = c.GetAsync(&uri).and_then(|op| op.join()).map_err(offline)?;
    let code = resp.StatusCode().map_err(offline)?;
    if code == HttpStatusCode::NotFound {
        return Ok(None);
    }
    if !resp.IsSuccessStatusCode().unwrap_or(false) {
        // 403 is GitHub's rate limit for unauthenticated requests (60 an hour).
        return Err(if code.0 == 403 { "GitHub is busy; try again in a while.".into() } else { format!("GitHub answered with error {}.", code.0) });
    }
    let buf = resp.Content().and_then(|c| c.ReadAsBufferAsync()).and_then(|op| op.join()).map_err(offline)?;
    let mut bytes = Array::<u8>::new();
    CryptographicBuffer::CopyToByteArray(&buf, &mut bytes).map_err(offline)?;
    Ok(Some(bytes.to_vec()))
}

fn sha256_hex(bytes: &[u8]) -> windows::core::Result<String> {
    let alg = HashAlgorithmProvider::OpenAlgorithm(&HashAlgorithmNames::Sha256()?)?;
    let hash = alg.HashData(&CryptographicBuffer::CreateFromByteArray(bytes)?)?;
    Ok(CryptographicBuffer::EncodeToHexString(&hash)?.to_string().to_ascii_lowercase())
}

/// `major.minor.patch`, ignoring a leading `v` and any `-pre` / `+build` suffix.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    Some((major, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("2", "1.99.99"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }

    #[test]
    fn hashes_sha256() {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn names_siblings() {
        assert_eq!(sibling(Path::new(r"C:\Apps\blazeaudio.exe"), ".old"), PathBuf::from(r"C:\Apps\blazeaudio.exe.old"));
    }
}
