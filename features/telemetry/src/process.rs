//! The process's own health as OpenTelemetry metrics, named by the
//! semantic conventions for processes: `process.cpu.time` (s, by
//! `cpu.mode`), `process.cpu.utilization` (1, over the CPUs the process
//! can use), `process.memory.usage` (By, resident) and
//! `process.thread.count`.
//!
//! All observable: read from the OS when the meter provider collects (an
//! export), never on a hot path. [`register`] is called by `otel::init`;
//! on a platform with nothing to read (not Linux or macOS) the
//! instruments report nothing.

use std::sync::Mutex;
use std::time::Instant;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Meter;

/// What the OS says about this process at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    /// CPU time spent in user and in system mode, seconds.
    pub user_s: f64,
    pub system_s: f64,
    /// Resident memory, bytes.
    pub resident_bytes: Option<u64>,
    /// Threads in the process.
    pub threads: Option<u64>,
}

impl Sample {
    /// CPU time in both modes, seconds.
    #[must_use]
    pub fn cpu_s(&self) -> f64 {
        self.user_s + self.system_s
    }
}

/// Read this process's [`Sample`]; `None` where there is no reader.
#[must_use]
pub fn sample() -> Option<Sample> {
    os::sample()
}

/// Register the process instruments on `meter`. Their callbacks run when
/// the provider collects.
pub fn register(meter: &Meter) {
    meter
        .f64_observable_counter("process.cpu.time")
        .with_unit("s")
        .with_description("CPU time the process has used, by mode")
        .with_callback(|observer| {
            if let Some(s) = sample() {
                observer.observe(s.user_s, &[KeyValue::new("cpu.mode", "user")]);
                observer.observe(s.system_s, &[KeyValue::new("cpu.mode", "system")]);
            }
        })
        .build();

    // Utilization since the collection before (or since registering):
    // CPU time over wall time over the CPUs the process can run on.
    let cpus = std::thread::available_parallelism().map_or(1.0, |n| {
        f64::from(u32::try_from(n.get()).unwrap_or(u32::MAX))
    });
    let last: Mutex<Option<(Instant, f64)>> =
        Mutex::new(sample().map(|s| (Instant::now(), s.cpu_s())));
    meter
        .f64_observable_gauge("process.cpu.utilization")
        .with_unit("1")
        .with_description(
            "CPU time over wall time since the last collection, over the CPUs available",
        )
        .with_callback(move |observer| {
            let Some(now) = sample() else { return };
            let at = Instant::now();
            let Ok(mut last) = last.lock() else { return };
            if let Some((then, cpu_then)) = *last {
                if let Some(ratio) = utilization(
                    now.cpu_s() - cpu_then,
                    at.saturating_duration_since(then),
                    cpus,
                ) {
                    observer.observe(ratio, &[]);
                }
            }
            *last = Some((at, now.cpu_s()));
        })
        .build();

    meter
        .i64_observable_up_down_counter("process.memory.usage")
        .with_unit("By")
        .with_description("Resident memory of the process")
        .with_callback(|observer| {
            if let Some(bytes) = sample().and_then(|s| s.resident_bytes) {
                observer.observe(i64::try_from(bytes).unwrap_or(i64::MAX), &[]);
            }
        })
        .build();

    meter
        .i64_observable_up_down_counter("process.thread.count")
        .with_unit("{thread}")
        .with_description("Threads in the process")
        .with_callback(|observer| {
            if let Some(threads) = sample().and_then(|s| s.threads) {
                observer.observe(i64::try_from(threads).unwrap_or(i64::MAX), &[]);
            }
        })
        .build();
}

/// `cpu_s` of CPU time over `wall` across `cpus`, 0..=1; `None` for no
/// wall time.
fn utilization(cpu_s: f64, wall: std::time::Duration, cores: f64) -> Option<f64> {
    let wall_s = wall.as_secs_f64();
    (wall_s > 0.0 && cores > 0.0).then(|| (cpu_s.max(0.0) / wall_s / cores).clamp(0.0, 1.0))
}

#[cfg(target_os = "linux")]
mod os {
    use super::Sample;

    /// Clock ticks a second in `/proc/self/stat`: `USER_HZ`, fixed at
    /// 100 by the kernel's userspace ABI.
    const USER_HZ: f64 = 100.0;

    pub fn sample() -> Option<Sample> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let status = std::fs::read_to_string("/proc/self/status").ok();
        let (user_s, system_s) = cpu_of_stat(&stat)?;
        let (resident_bytes, threads) = status.as_deref().map_or((None, None), memory_of_status);
        Some(Sample {
            user_s,
            system_s,
            resident_bytes,
            threads,
        })
    }

    /// `utime` and `stime` (fields 14 and 15) of `/proc/<pid>/stat`, in
    /// seconds. The command name (field 2) may hold spaces and parens, so
    /// the fields are counted from its closing paren.
    pub fn cpu_of_stat(stat: &str) -> Option<(f64, f64)> {
        let after = stat.get(stat.rfind(')')?.checked_add(1)?..)?;
        let mut fields = after.split_whitespace().skip(11);
        let utime: u64 = fields.next()?.parse().ok()?;
        let stime: u64 = fields.next()?.parse().ok()?;
        let secs = |ticks: u64| f64::from(u32::try_from(ticks).unwrap_or(u32::MAX)) / USER_HZ;
        Some((secs(utime), secs(stime)))
    }

    /// `VmRSS` (bytes) and `Threads` of `/proc/<pid>/status`.
    pub fn memory_of_status(status: &str) -> (Option<u64>, Option<u64>) {
        let value = |key: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
        };
        (
            value("VmRSS:").map(|kib| kib.saturating_mul(1024)),
            value("Threads:"),
        )
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::Sample;

    pub fn sample() -> Option<Sample> {
        // SAFETY: getrusage and proc_pidinfo write into the zeroed
        // structs passed, of the sizes passed, and nothing else.
        let usage = unsafe {
            let mut usage: libc::rusage = core::mem::zeroed();
            (libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) == 0).then_some(usage)
        }?;
        let task = unsafe {
            let mut info: libc::proc_taskinfo = core::mem::zeroed();
            let size = i32::try_from(core::mem::size_of::<libc::proc_taskinfo>()).ok()?;
            let wrote = libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                (&raw mut info).cast(),
                size,
            );
            (wrote == size).then_some(info)
        };
        let secs = |t: libc::timeval| {
            f64::from(u32::try_from(t.tv_sec).unwrap_or(u32::MAX)) + f64::from(t.tv_usec) / 1e6
        };
        Some(Sample {
            user_s: secs(usage.ru_utime),
            system_s: secs(usage.ru_stime),
            resident_bytes: task.map(|t| t.pti_resident_size),
            threads: task.and_then(|t| u64::try_from(t.pti_threadnum).ok()),
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod os {
    use super::Sample;

    pub const fn sample() -> Option<Sample> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utilization_is_cpu_over_wall_over_cpus() {
        let second = std::time::Duration::from_secs(1);
        assert_eq!(utilization(1.0, second, 4.0), Some(0.25));
        assert_eq!(utilization(-1.0, second, 4.0), Some(0.0));
        assert_eq!(utilization(1.0, std::time::Duration::ZERO, 4.0), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_proc_files_parse() {
        // A command name with a space and a paren in it.
        let stat =
            "4242 (my (app) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 250 75 0 0 20 0 7 0 1 2 3";
        assert_eq!(os::cpu_of_stat(stat), Some((2.5, 0.75)));
        let status = "Name:\tx\nVmRSS:\t  2048 kB\nThreads:\t7\n";
        assert_eq!(os::memory_of_status(status), (Some(2048 * 1024), Some(7)));
        let live = sample().expect("this process");
        assert!(live.resident_bytes.is_some_and(|b| b > 0));
        assert!(live.threads.is_some_and(|t| t >= 1));
    }
}
