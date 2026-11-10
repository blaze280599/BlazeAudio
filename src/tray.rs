//! The notification-area (tray) icon, the app's home while its window is hidden. A click shows or
//! hides the window; the right-click menu also switches between beside Spotify and standalone,
//! and quits. The icon talks to a hidden window of our own, whose messages arrive on the main
//! thread through winit's message loop; they reach the app as `UserEvent::Tray`.

use std::cell::{Cell, RefCell};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuRadioItem, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, GetCursorPos,
    GetSystemMetrics, LoadImageW, PostMessageW, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, TrackPopupMenuEx,
    HICON, IMAGE_ICON, LR_DEFAULTCOLOR, MF_BYCOMMAND, MF_SEPARATOR, MF_STRING, SM_CXSMICON, SM_CYSMICON, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_NULL, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

/// What the user asked for through the tray icon.
#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    /// A click on the icon: show the window, or hide it if it was in use just now.
    Click,
    Show,
    Hide,
    /// Beside Spotify (true) or standalone.
    Docked(bool),
    Quit,
}

/// The message the icon sends its window.
const CALLBACK: u32 = WM_APP + 1;
/// Keyboard selection (Enter or Space on the focused icon); `NIN_SELECT | NINF_KEY`.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

const ID_SHOW: u32 = 1;
const ID_HIDE: u32 = 2;
const ID_DOCKED: u32 = 3;
const ID_STANDALONE: u32 = 4;
const ID_QUIT: u32 = 5;

thread_local! {
    /// The icon's window runs on the main thread, so its state lives here.
    static PROXY: RefCell<Option<EventLoopProxy<UserEvent>>> = const { RefCell::new(None) };
    /// The icon as last added or modified, to add it again when Explorer restarts.
    static DATA: RefCell<Option<NOTIFYICONDATAW>> = const { RefCell::new(None) };
    /// What the menu offers: the window is on screen, docking is on.
    static STATE: Cell<(bool, bool)> = const { Cell::new((true, false)) };
    /// Explorer broadcasts this after it restarts, when every tray icon has to be added again.
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// The icon in the notification area; removed on drop.
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    /// Adds the icon. `None` when Windows refuses (no shell, say); the app then quits on close.
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Option<Self> {
        unsafe {
            let inst = GetModuleHandleW(None).ok()?;
            let class = w!("BlazeAudio.Tray");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            if RegisterClassW(&wc) == 0 {
                return None;
            }
            // A hidden top-level window rather than a message-only one: only top-level windows get
            // the TaskbarCreated broadcast, and the menu needs a window that can take the foreground.
            let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("BlazeAudio"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None).ok()?;
            TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
            PROXY.with(|p| *p.borrow_mut() = Some(proxy));
            // Icon group 1 is the exe's icon (see build.rs), here at the small-icon size.
            let icon = LoadImageW(Some(inst.into()), PCWSTR(1 as _), IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR)
                .map(|h| HICON(h.0))
                .unwrap_or_default();
            let mut d = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
                uCallbackMessage: CALLBACK,
                hIcon: icon,
                ..Default::default()
            };
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            put_tip(&mut d, "BlazeAudio");
            DATA.with(|s| *s.borrow_mut() = Some(d));
            if !add() {
                let _ = DestroyWindow(hwnd);
                let _ = DestroyIcon(icon);
                return None;
            }
            Some(Self { hwnd, icon })
        }
    }

    /// The icon's tooltip; the track playing, like the window title.
    pub fn set_tip(&self, text: &str) {
        DATA.with(|s| {
            if let Some(d) = &mut *s.borrow_mut() {
                put_tip(d, text);
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_MODIFY, d);
                }
            }
        });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        DATA.with(|s| {
            if let Some(d) = s.borrow_mut().take() {
                unsafe {
                    let _ = Shell_NotifyIconW(NIM_DELETE, &d);
                }
            }
        });
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = DestroyIcon(self.icon);
        }
    }
}

/// What the menu offers next time it opens: Show or Hide, and which mode is ticked. Cheap; the
/// app calls it whenever it schedules a frame.
pub fn set_state(on_screen: bool, docked: bool) {
    STATE.set((on_screen, docked));
}

fn put_tip(d: &mut NOTIFYICONDATAW, text: &str) {
    d.szTip = [0; 128];
    for (dst, c) in d.szTip.iter_mut().zip(text.encode_utf16().take(127)) {
        *dst = c;
    }
}

/// Adds the icon from `DATA` and opts into the version-4 messages `wndproc` expects.
fn add() -> bool {
    DATA.with(|s| {
        let Some(d) = &*s.borrow() else { return false };
        unsafe { Shell_NotifyIconW(NIM_ADD, d).as_bool() && Shell_NotifyIconW(NIM_SETVERSION, d).as_bool() }
    })
}

fn send(cmd: Cmd) {
    PROXY.with(|p| {
        if let Some(p) = &*p.borrow() {
            let _ = p.send_event(UserEvent::Tray(cmd));
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        CALLBACK => {
            // Version 4: the event is in the low word of lParam.
            match lp.0 as u32 & 0xffff {
                NIN_SELECT | NIN_KEYSELECT => send(Cmd::Click),
                WM_CONTEXTMENU => menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        // `taskkill` without /F closes every top-level window of the process; this one quits.
        WM_CLOSE => {
            send(Cmd::Quit);
            return LRESULT(0);
        }
        m if m != 0 && m == TASKBAR_CREATED.get() => {
            add();
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// The right-click menu. Runs its own modal loop here, outside the app's event handler, so the
/// scene keeps animating while it's open.
fn menu(hwnd: HWND) {
    let (on_screen, docked) = STATE.get();
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let (first, label) = if on_screen { (ID_HIDE, w!("Hide BlazeAudio")) } else { (ID_SHOW, w!("Show BlazeAudio")) };
        let _ = AppendMenuW(m, MF_STRING, first as usize, label);
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, ID_DOCKED as usize, w!("Beside Spotify"));
        let _ = AppendMenuW(m, MF_STRING, ID_STANDALONE as usize, w!("Standalone"));
        let _ = CheckMenuRadioItem(m, ID_DOCKED, ID_STANDALONE, if docked { ID_DOCKED } else { ID_STANDALONE }, MF_BYCOMMAND.0);
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(m, MF_STRING, ID_QUIT as usize, w!("Quit"));
        let _ = SetMenuDefaultItem(m, first, 0);
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Without the foreground the menu wouldn't close on a click elsewhere; the WM_NULL after
        // it is the documented companion to that.
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenuEx(m, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0, p.x, p.y, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        match id {
            ID_SHOW => send(Cmd::Show),
            ID_HIDE => send(Cmd::Hide),
            ID_DOCKED => send(Cmd::Docked(true)),
            ID_STANDALONE => send(Cmd::Docked(false)),
            ID_QUIT => send(Cmd::Quit),
            _ => {}
        }
    }
}
