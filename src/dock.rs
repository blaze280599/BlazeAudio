//! Docking beside Spotify: finds Spotify's main window, follows it with out-of-context WinEvent
//! hooks (delivered on the main thread through winit's message loop, so nothing polls while it
//! sits still), and moves BlazeAudio's window flush against one of its sides.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, COLORREF, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_ROUND,
    DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, SetWindowPos, ShowWindow, CHILDID_SELF, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GW_HWNDPREV, GW_OWNER, HWND_TOP, OBJID_WINDOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WINEVENT_OUTOFCONTEXT,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, ANIMATIONINFO, LWA_ALPHA, SPI_GETANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetLayeredWindowAttributes, SetWindowLongW,
    SystemParametersInfoW, WS_EX_LAYERED,
};
use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::UserEvent;

thread_local! {
    /// Hook callbacks run on the main thread (the one that set the hooks), so the proxy lives here.
    static PROXY: RefCell<Option<EventLoopProxy<UserEvent>>> = const { RefCell::new(None) };
}
/// The followed window's handle, for the hook callback's filter.
static TARGET: AtomicIsize = AtomicIsize::new(0);
/// A `UserEvent::Dock` is queued and not yet handled; a window drag fires hundreds of events, one wake covers them.
static PENDING: AtomicBool = AtomicBool::new(false);

/// How often to look for Spotify's window while docking is on and it isn't found.
const SEARCH_EVERY: Duration = Duration::from_secs(2);

pub fn init(proxy: EventLoopProxy<UserEvent>) {
    PROXY.with(|p| *p.borrow_mut() = Some(proxy));
}

/// Marks the queued wake as handled, so the next event queues another.
pub fn clear_pending() {
    PENDING.store(false, Ordering::Relaxed);
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, _event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    // Spotify's renderer fires location and focus events for its own UI objects too.
    if hwnd.0 as isize != TARGET.load(Ordering::Relaxed) || obj != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 {
        return;
    }
    if !PENDING.swap(true, Ordering::Relaxed) {
        PROXY.with(|p| {
            if let Some(p) = &*p.borrow() {
                let _ = p.send_event(UserEvent::Dock);
            }
        });
    }
}

/// The Spotify window being followed and the hooks on its process.
pub struct Tracker {
    target: Option<HWND>,
    hooks: Vec<HWINEVENTHOOK>,
    last_search: Option<Instant>,
}

impl Tracker {
    pub fn new() -> Self {
        Self { target: None, hooks: Vec::new(), last_search: None }
    }

    /// Spotify's main window while it's open and shown. Without one, looks for it again (at most
    /// every couple of seconds) and hooks the new one's events.
    pub fn target(&mut self) -> Option<HWND> {
        if let Some(h) = self.target {
            if unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() } {
                return Some(h);
            }
            self.release();
        }
        if self.last_search.is_some_and(|t| t.elapsed() < SEARCH_EVERY) {
            return None;
        }
        self.last_search = Some(Instant::now());
        let h = find_spotify()?;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        TARGET.store(h.0 as isize, Ordering::Relaxed);
        for (lo, hi) in [
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            // Destroy, show and hide are consecutive.
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
        ] {
            let hook = unsafe { SetWinEventHook(lo, hi, None, Some(on_event), pid, 0, WINEVENT_OUTOFCONTEXT) };
            if !hook.is_invalid() {
                self.hooks.push(hook);
            }
        }
        self.target = Some(h);
        #[cfg(debug_assertions)]
        eprintln!("[dock] following Spotify window {:?} (pid {pid}, {} hooks)", h.0, self.hooks.len());
        Some(h)
    }

    /// The window being followed, without checking it's still there.
    pub fn current(&self) -> Option<HWND> {
        self.target
    }

    pub fn following(&self) -> bool {
        self.target.is_some()
    }

    /// The followed window closed or hid without an event reaching the hooks (Spotify killed or
    /// crashed, say). Cheap enough to ask every frame.
    pub fn lost(&self) -> bool {
        self.target.is_some_and(|h| unsafe { !IsWindow(Some(h)).as_bool() || !IsWindowVisible(h).as_bool() })
    }

    /// Stops following: unhooks and forgets the window. Looks again on the next `target()`.
    pub fn release(&mut self) {
        for h in self.hooks.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
        if self.target.take().is_some() {
            self.last_search = None;
        }
        TARGET.store(0, Ordering::Relaxed);
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.release();
    }
}

/// Where Spotify's window is and what state it's in, in physical pixels.
pub struct TargetInfo {
    /// Visible bounds (without the invisible resize border).
    pub rect: RECT,
    /// Work area of the monitor it's on.
    pub work: RECT,
    pub minimized: bool,
    /// Maximized, or covering its whole monitor: no room beside it.
    pub fills: bool,
    pub foreground: bool,
}

pub fn info(target: HWND) -> Option<TargetInfo> {
    unsafe {
        let rect = visible_rect(target)?;
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        let m = mi.rcMonitor;
        let covers = rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom;
        Some(TargetInfo {
            rect,
            work: mi.rcWork,
            minimized: IsIconic(target).as_bool(),
            fills: IsZoomed(target).as_bool() || covers,
            foreground: GetForegroundWindow() == target,
        })
    }
}

/// The window's HWND, from winit.
pub fn hwnd(w: &Window) -> Option<HWND> {
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Visible bounds of a window: DWM's frame bounds, which leave out the invisible resize border
/// that `GetWindowRect` includes.
fn visible_rect(h: HWND) -> Option<RECT> {
    unsafe {
        let mut r = RECT::default();
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, size_of::<RECT>() as u32).is_ok() && r.right > r.left {
            return Some(r);
        }
        GetWindowRect(h, &mut r).ok().map(|_| r)
    }
}

/// Moves `ours` so its visible bounds are (x, y, w, h), without activating it or changing the
/// z-order. Does nothing when it's already there.
pub fn place(ours: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(ours, &mut wr).is_err() {
            return;
        }
        let vis = visible_rect(ours).unwrap_or(wr);
        let (dl, dt, dr, db) = (vis.left - wr.left, vis.top - wr.top, wr.right - vis.right, wr.bottom - vis.bottom);
        let (ox, oy, ow, oh) = (x - dl, y - dt, w + dl + dr, h + dt + db);
        if (ox, oy, ow, oh) == (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top) {
            return;
        }
        let _ = SetWindowPos(ours, None, ox, oy, ow, oh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
}

/// Puts `ours` on top of the z-order (just above Spotify, which just came to the front) without
/// taking focus, so the panel isn't left behind other windows.
pub fn raise(ours: HWND) {
    unsafe {
        let _ = SetWindowPos(ours, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Puts `ours` directly above Spotify's window in the z-order without taking focus, so the panel
/// is exactly as covered as Spotify is: in front with it, behind whatever is in front of it.
/// Docking from the background (launched from a terminal, say) would otherwise leave the panel
/// wherever Windows put the new window, under a game or editor that Spotify is in front of.
pub fn stack_above(target: HWND, ours: HWND) {
    unsafe {
        let prev = GetWindow(target, GW_HWNDPREV).ok();
        if prev == Some(ours) {
            return;
        }
        // Going in after a topmost window would make the panel topmost too; with one right above
        // Spotify, Spotify is the first ordinary window, so the top of that band is the same place.
        let after = prev.filter(|&h| GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 == 0).unwrap_or(HWND_TOP);
        let _ = SetWindowPos(ours, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
}

/// Puts Spotify's window right under `ours`, so clicking the panel brings Spotify up with it.
pub fn raise_target_under(target: HWND, ours: HWND) {
    unsafe {
        let _ = SetWindowPos(target, Some(ours), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// Fades a window: `Some(a)` makes it layered at that opacity (0..1). `None` drops the layered
/// style again, which matters: a layered window shows a held frame instead of the swapchain's
/// live ones, so it's only layered while a fade runs.
pub fn set_alpha(h: HWND, a: Option<f32>) {
    unsafe {
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let layered = ex & WS_EX_LAYERED.0 != 0;
        match a {
            Some(a) => {
                if !layered {
                    SetWindowLongW(h, GWL_EXSTYLE, (ex | WS_EX_LAYERED.0) as i32);
                }
                let _ = SetLayeredWindowAttributes(h, COLORREF(0), (a.clamp(0.0, 1.0) * 255.0).round() as u8, LWA_ALPHA);
            }
            None if layered => {
                SetWindowLongW(h, GWL_EXSTYLE, (ex & !WS_EX_LAYERED.0) as i32);
            }
            None => {}
        }
    }
}

/// The Windows setting "Animate windows when minimizing and maximizing", which the panel's fade
/// stands in for; with it off, the panel just hides and shows.
pub fn animations_on() -> bool {
    let mut ai = ANIMATIONINFO { cbSize: size_of::<ANIMATIONINFO>() as u32, iMinAnimate: 1 };
    let ok = unsafe { SystemParametersInfoW(SPI_GETANIMATION, ai.cbSize, Some(&mut ai as *mut ANIMATIONINFO as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok() };
    !ok || ai.iMinAnimate != 0
}

/// Asks DWM to round the window's corners like other Windows 11 apps. A frameless window gets
/// square corners by default; `false` goes back to the system's choice. Older Windows ignores it.
pub fn round_corners(ours: HWND, round: bool) {
    let pref = if round { DWMWCP_ROUND } else { DWMWCP_DEFAULT };
    unsafe {
        let _ = DwmSetWindowAttribute(ours, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32);
    }
}

pub fn hide(ours: HWND) {
    unsafe {
        let _ = ShowWindow(ours, SW_HIDE);
    }
}

/// Shows (and un-minimizes) a window without activating it: the panel without taking focus
/// from Spotify, or Spotify when the panel is brought back by hand.
pub fn show(h: HWND) {
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
    }
}

/// The window is shown: not hidden, though it may be minimized.
pub fn visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h).as_bool() }
}

/// The primary mouse button is held: an edge drag started with `drag_resize_window` is still going.
pub fn left_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 }
}

/// Spotify's main window: a shown, unowned, non-tool Chromium window with a title, belonging to
/// Spotify.exe (the spotify.com install or the Microsoft Store one). The largest wins.
fn find_spotify() -> Option<HWND> {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let found = unsafe { &mut *(lp.0 as *mut Vec<HWND>) };
        if is_spotify_main(h) {
            found.push(h);
        }
        true.into()
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut Vec<HWND> as isize));
    }
    found.into_iter().max_by_key(|h| {
        let r = visible_rect(*h).unwrap_or_default();
        (r.right - r.left) as i64 * (r.bottom - r.top) as i64
    })
}

fn is_spotify_main(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() || GetWindow(h, GW_OWNER).is_ok() || GetWindowTextLengthW(h) == 0 {
            return false;
        }
        if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut class = [0u16; 32];
        let n = GetClassNameW(h, &mut class) as usize;
        if String::from_utf16_lossy(&class[..n]) != "Chrome_WidgetWin_1" {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(proc);
        ok && String::from_utf16_lossy(&buf[..len as usize]).rsplit('\\').next().is_some_and(|f| f.eq_ignore_ascii_case("spotify.exe"))
    }
}
