//! Small Win32 helpers: global hotkey polling, process CPU/RAM stats, working-set trim.

use std::time::Instant;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GlobalKey {
    ToggleOverlay,
    ToggleSettings,
}

/// Polls Ctrl+Alt+{O,S} regardless of focus — needed because the overlay is click-through.
pub struct GlobalHotkeys {
    prev: [bool; 2],
}

impl GlobalHotkeys {
    pub fn new() -> Self {
        Self { prev: [false; 2] }
    }

    pub fn poll(&mut self) -> Option<GlobalKey> {
        let down = |vk: i32| unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 };
        let mods = down(VK_CONTROL) && down(VK_MENU);
        let keys = [(b'O', GlobalKey::ToggleOverlay), (b'S', GlobalKey::ToggleSettings)];
        let mut fired = None;
        for (i, (vk, k)) in keys.iter().enumerate() {
            let now = mods && down(*vk as i32);
            if now && !self.prev[i] {
                fired = Some(*k);
            }
            self.prev[i] = now;
        }
        fired
    }
}

pub struct ProcStats {
    last_wall: Instant,
    last_cpu: u64,
    pub cpu_percent: f32,
    pub working_set_mb: f32,
    pub private_mb: f32,
    cores: f32,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn cpu_time_100ns() -> u64 {
    unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_ok() {
            ft(k) + ft(u)
        } else {
            0
        }
    }
}

impl ProcStats {
    pub fn new() -> Self {
        Self {
            last_wall: Instant::now(),
            last_cpu: cpu_time_100ns(),
            cpu_percent: 0.0,
            working_set_mb: 0.0,
            private_mb: 0.0,
            cores: std::thread::available_parallelism().map(|n| n.get() as f32).unwrap_or(1.0),
        }
    }

    /// Refresh at most once a second. CPU% is of total machine capacity (like Task Manager).
    pub fn update(&mut self) {
        let el = self.last_wall.elapsed().as_secs_f64();
        if el < 1.0 {
            return;
        }
        let cpu = cpu_time_100ns();
        let used = (cpu.saturating_sub(self.last_cpu)) as f64 * 1e-7;
        self.cpu_percent = (used / el / self.cores as f64 * 100.0) as f32;
        self.last_cpu = cpu;
        self.last_wall = Instant::now();
        unsafe {
            let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pmc.cb).as_bool() {
                self.working_set_mb = pmc.WorkingSetSize as f32 / (1024.0 * 1024.0);
                self.private_mb = pmc.PrivateUsage as f32 / (1024.0 * 1024.0);
            }
        }
    }
}

/// Drop pages touched only during startup (shader compilation, decoder init) from the working set.
pub fn trim_working_set() {
    unsafe {
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

/// winit makes a window click-through by adding WS_EX_LAYERED | WS_EX_TRANSPARENT. A layered
/// window is never shown until SetLayeredWindowAttributes is called, so give it full opacity
/// (per-pixel transparency still comes from the DirectComposition swapchain).
pub fn fix_layered(window: &winit::window::Window) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, SetLayeredWindowAttributes, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = HWND(h.hwnd.get() as *mut core::ffi::c_void);
    unsafe {
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0 != 0 {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }
    }
}
