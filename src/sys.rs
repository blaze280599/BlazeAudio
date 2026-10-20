//! Small Win32 helpers: process CPU/RAM stats, working-set trim.

use std::time::Instant;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

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
