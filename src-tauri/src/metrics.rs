//! Resource telemetry: Windows host (via `sysinfo`) + Linux guest (via one `sh` call
//! that reads /proc, `df` and `ps`, portable across procps and busybox distros).
//!
//! Every wsl.exe call goes through the helpers in `wsl.rs` (CREATE_NO_WINDOW, UTF-8,
//! timeouts). Guest commands never run against a stopped distro, since `wsl -d` would
//! boot it as a side effect.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::System;

use crate::wsl::{ensure_running, is_running, run, sh_in_distro, validate_distro_name};

const GUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Processes returned to the UI, heaviest RSS first. The UI shows the top 8 and its
/// search filters across all of these.
const PROCESS_LIMIT: usize = 25;
/// Host process of the WSL2 utility VM (`vmmem` on older WSL builds), in order of preference.
const VM_PROCESS_NAMES: [&str; 2] = ["vmmemwsl.exe", "vmmem.exe"];
/// The rest of WSL's footprint on Windows: the service, per-distro hosts, port relays and
/// wsl.exe clients (this app's terminals and probes included).
const WSL_HELPER_NAMES: [&str; 4] = ["wslservice.exe", "wslhost.exe", "wslrelay.exe", "wsl.exe"];

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/wsl.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslResourceMetrics {
    pub distro: String,
    pub host: HostMetrics,
    /// `None` when the distro is not running.
    pub guest: Option<GuestMetrics>,
    /// Set when the distro is running but the guest probe failed.
    pub guest_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostMetrics {
    pub total_memory: u64,
    pub used_memory: u64,
    /// Global host CPU usage, 0-100.
    pub cpu_usage: f32,
    pub cpu_count: usize,
    /// The utility VM; `None` while WSL is shut down.
    pub vm: Option<VmProcess>,
    /// wslservice/wslhost/wslrelay/wsl.exe, summed (they exist even with the VM off).
    pub wsl_helpers: WslHelpers,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmProcess {
    pub name: String,
    pub pid: u32,
    /// Private working set in bytes: Task Manager's "Memory" column.
    pub memory: u64,
    /// CPU usage normalized to the whole host, 0-100.
    pub cpu_usage: f32,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WslHelpers {
    /// Private working set in bytes, summed.
    pub memory: u64,
    pub processes: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestMetrics {
    pub memory: GuestMemory,
    pub load: LoadAverage,
    /// CPU usage of the WSL kernel (shared by all distros), 0-100. `None` on the first sample.
    pub cpu_usage: Option<f32>,
    pub cpu_count: u32,
    pub disk: Option<DiskUsage>,
    pub processes: Vec<LinuxProcess>,
}

/// All values in bytes. `used` excludes reclaimable cache, like `free(1)`.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuestMemory {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub available: u64,
    /// Buffers + page cache + reclaimable slab.
    pub cache: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoadAverage {
    pub one: f32,
    pub five: f32,
    pub fifteen: f32,
    pub running: u32,
    pub total: u32,
}

/// All values in bytes, for the filesystem mounted at `/` (the distro's ext4.vhdx).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub filesystem: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinuxProcess {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    /// procps `%CPU` (lifetime average). `None` on busybox, whose `ps` has no CPU column.
    pub cpu: Option<f32>,
    /// Resident set size in bytes.
    pub rss: u64,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropCachesResult {
    pub cache_before: u64,
    pub cache_after: u64,
    pub free_before: u64,
    pub free_after: u64,
}

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

/// Long-lived `sysinfo::System`: CPU usage is a delta between refreshes, so the same
/// instance must be reused across calls.
#[derive(Default)]
pub struct HostMonitor(Arc<Mutex<HostState>>);

#[derive(Default)]
struct HostState {
    sys: System,
    /// Previous (pid, CPU time in 100ns units, instant) of the VM process.
    vm_cpu: Option<(u32, u64, Instant)>,
}

/// Previous `/proc/stat` sample per distro, to compute guest CPU usage as a delta.
#[derive(Default)]
pub struct GuestSampler(Mutex<HashMap<String, CpuTimes>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

// ---------------------------------------------------------------------------
// Host (Windows)
// ---------------------------------------------------------------------------

/// Picks WSL's processes out of a full process list: the VM (first match in
/// `VM_PROCESS_NAMES` order) plus the helpers, summed. Names are compared case-insensitively.
fn summarize_wsl(entries: &[nt::ProcessEntry]) -> (Option<&nt::ProcessEntry>, WslHelpers) {
    let vm = VM_PROCESS_NAMES
        .iter()
        .find_map(|wanted| entries.iter().find(|e| e.name.eq_ignore_ascii_case(wanted)));
    let mut helpers = WslHelpers::default();
    for e in entries.iter().filter(|e| WSL_HELPER_NAMES.iter().any(|h| e.name.eq_ignore_ascii_case(h))) {
        helpers.memory += e.memory();
        helpers.processes += 1;
    }
    (vm, helpers)
}

impl HostState {
    fn sample(&mut self) -> HostMetrics {
        let sys = &mut self.sys;
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        let cpu_count = sys.cpus().len().max(1);

        // One kernel snapshot of every process, re-read on each sample: the VM appears and
        // disappears with `wsl --shutdown`, so nothing is cached by PID. It also works for
        // vmmemWSL, a protected process OpenProcess can't open without admin.
        let entries = nt::snapshot();
        let (vm_entry, wsl_helpers) = summarize_wsl(&entries);
        let now = Instant::now();
        let vm = vm_entry.map(|e| {
            let cpu_usage = match self.vm_cpu {
                Some((prev_pid, prev_time, at)) if prev_pid == e.pid => {
                    let elapsed = now.duration_since(at).as_nanos() as f64 / 100.0;
                    let used = e.cpu_time.saturating_sub(prev_time) as f64;
                    (used / (elapsed * cpu_count as f64) * 100.0).clamp(0.0, 100.0) as f32
                }
                _ => 0.0,
            };
            VmProcess { name: e.display_name.clone(), pid: e.pid, memory: e.memory(), cpu_usage }
        });
        self.vm_cpu = vm_entry.map(|e| (e.pid, e.cpu_time, now));

        HostMetrics {
            total_memory: sys.total_memory(),
            used_memory: sys.used_memory(),
            cpu_usage: sys.global_cpu_usage(),
            cpu_count,
            vm,
            wsl_helpers,
        }
    }
}

/// Process list from `NtQuerySystemInformation(SystemProcessInformation)`, which (unlike
/// OpenProcess) also covers protected processes without elevation.
mod nt {
    pub struct ProcessEntry {
        pub pid: u32,
        /// Image name as Windows reports it, e.g. `vmmemWSL` / `wslservice.exe`.
        pub display_name: String,
        /// Lowercase, always with `.exe` (minimal processes like vmmemWSL report no suffix).
        pub name: String,
        pub working_set: u64,
        pub private_working_set: u64,
        /// User + kernel time, in 100ns units.
        pub cpu_time: u64,
    }

    impl ProcessEntry {
        /// Task Manager's "Memory" column (private working set), falling back to the full
        /// working set when the kernel doesn't report the private part.
        pub fn memory(&self) -> u64 {
            if self.private_working_set > 0 { self.private_working_set } else { self.working_set }
        }
    }

    pub fn normalize_name(image: &str) -> String {
        let lower = image.to_ascii_lowercase();
        if lower.ends_with(".exe") { lower } else { format!("{lower}.exe") }
    }

    #[cfg(windows)]
    pub fn snapshot() -> Vec<ProcessEntry> {
        use ntapi::ntexapi::{
            NtQuerySystemInformation, SystemProcessInformation, SYSTEM_PROCESS_INFORMATION,
        };
        const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

        // u64 storage keeps the buffer 8-byte aligned for SYSTEM_PROCESS_INFORMATION.
        let mut buf: Vec<u64> = vec![0; 64 * 1024];
        loop {
            let size = (buf.len() * 8) as u32;
            let mut needed = 0u32;
            // SAFETY: buf is valid for `size` bytes; the kernel writes at most that much.
            let status = unsafe {
                NtQuerySystemInformation(
                    SystemProcessInformation,
                    buf.as_mut_ptr().cast(),
                    size,
                    &mut needed,
                )
            };
            match status {
                STATUS_INFO_LENGTH_MISMATCH => buf.resize(needed as usize / 8 + 16 * 1024, 0),
                s if s < 0 => return Vec::new(),
                _ => break,
            }
        }

        let base = buf.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        let mut entries = Vec::new();
        loop {
            // SAFETY: entries are a kernel-built, 8-aligned linked list inside `buf`,
            // chained by NextEntryOffset and terminated by 0.
            let info = unsafe { &*base.add(offset).cast::<SYSTEM_PROCESS_INFORMATION>() };
            let image = &info.ImageName;
            let display_name = if image.Buffer.is_null() || image.Length == 0 {
                String::new() // the Idle process
            } else {
                // SAFETY: Buffer points to Length bytes of UTF-16 inside the same buffer.
                let units = unsafe { std::slice::from_raw_parts(image.Buffer, image.Length as usize / 2) };
                String::from_utf16_lossy(units)
            };
            // SAFETY: LARGE_INTEGER is a plain i64 union.
            let (user, kernel, private) = unsafe {
                (*info.UserTime.QuadPart(), *info.KernelTime.QuadPart(), *info.WorkingSetPrivateSize.QuadPart())
            };
            entries.push(ProcessEntry {
                pid: info.UniqueProcessId as usize as u32,
                name: normalize_name(&display_name),
                display_name,
                working_set: info.WorkingSetSize as u64,
                private_working_set: private.max(0) as u64,
                cpu_time: (user + kernel) as u64,
            });
            if info.NextEntryOffset == 0 {
                return entries;
            }
            offset += info.NextEntryOffset as usize;
        }
    }

    #[cfg(not(windows))]
    pub fn snapshot() -> Vec<ProcessEntry> {
        Vec::new()
    }
}

impl HostMonitor {
    async fn sample(&self) -> Result<HostMetrics, String> {
        let state = Arc::clone(&self.0);
        tauri::async_runtime::spawn_blocking(move || {
            state.lock().unwrap_or_else(|e| e.into_inner()).sample()
        })
        .await
        .map_err(|e| format!("Falha ao coletar métricas do Windows: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Guest (Linux) — parsing
// ---------------------------------------------------------------------------

/// One round trip: sections are delimited by `@@name [arg]` lines. `ps` flavor is
/// detected in-script; procps gets `%CPU`, busybox does not have it.
const GUEST_SCRIPT: &str = r#"echo @@meminfo; cat /proc/meminfo
echo @@loadavg; cat /proc/loadavg
echo @@stat; head -n 1 /proc/stat
echo @@nproc; grep -c ^processor /proc/cpuinfo
echo @@df; df -kP / 2>/dev/null
echo @@self $$
if ps --version 2>/dev/null | grep -q procps; then
  echo @@ps procps; ps -ww -eo pid,ppid,user,pcpu,rss,args
else
  echo @@ps busybox; ps -o pid,ppid,user,rss,args
fi"#;

struct Section<'a> {
    arg: &'a str,
    lines: Vec<&'a str>,
}

fn split_sections(text: &str) -> HashMap<&str, Section<'_>> {
    let mut sections = HashMap::new();
    let mut current: Option<(&str, Section)> = None;
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("@@") {
            if let Some((name, section)) = current.take() {
                sections.insert(name, section);
            }
            let (name, arg) = header.split_once(' ').unwrap_or((header, ""));
            current = Some((name.trim(), Section { arg: arg.trim(), lines: Vec::new() }));
        } else if let Some((_, section)) = current.as_mut() {
            section.lines.push(line);
        }
    }
    if let Some((name, section)) = current {
        sections.insert(name, section);
    }
    sections
}

/// Parses `/proc/meminfo` (values in kB) into bytes.
pub fn parse_meminfo(lines: &[&str]) -> GuestMemory {
    let fields: HashMap<&str, u64> = lines
        .iter()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key.trim(), kb * 1024))
        })
        .collect();
    let get = |key: &str| fields.get(key).copied().unwrap_or(0);

    let total = get("MemTotal");
    let free = get("MemFree");
    let cache = get("Buffers") + get("Cached") + get("SReclaimable");
    // MemAvailable is missing on very old kernels; estimate it like free(1) used to.
    let available = fields.get("MemAvailable").copied().unwrap_or(free + cache);
    GuestMemory {
        total,
        used: total.saturating_sub(free).saturating_sub(cache),
        free,
        available,
        cache,
    }
}

/// Parses `/proc/loadavg`: `0.52 0.58 0.59 2/1234 5678`.
pub fn parse_loadavg(line: &str) -> Option<LoadAverage> {
    let mut it = line.split_whitespace();
    let one = it.next()?.parse().ok()?;
    let five = it.next()?.parse().ok()?;
    let fifteen = it.next()?.parse().ok()?;
    let (running, total) = it.next()?.split_once('/')?;
    Some(LoadAverage {
        one,
        five,
        fifteen,
        running: running.parse().ok()?,
        total: total.parse().ok()?,
    })
}

/// Parses the aggregate `cpu` line of `/proc/stat`.
fn parse_cpu_times(line: &str) -> Option<CpuTimes> {
    let mut it = line.split_whitespace();
    if it.next()? != "cpu" {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already in user).
    let values: Vec<u64> = it.take(8).filter_map(|v| v.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some(CpuTimes { busy: total - idle, total })
}

fn cpu_usage_between(prev: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(prev.total)?;
    let busy = now.busy.checked_sub(prev.busy)?;
    (total > 0).then(|| (busy as f32 / total as f32 * 100.0).clamp(0.0, 100.0))
}

/// Parses `df -kP /`. Read from the end of the data row, since the filesystem name
/// may contain spaces: `FS... total used avail capacity mount`.
pub fn parse_df(lines: &[&str]) -> Option<DiskUsage> {
    let row = lines.iter().skip(1).find(|l| !l.trim().is_empty())?;
    let tokens: Vec<&str> = row.split_whitespace().collect();
    if tokens.len() < 6 {
        return None;
    }
    let n = tokens.len();
    let kb = |s: &str| s.parse::<u64>().ok().map(|v| v * 1024);
    Some(DiskUsage {
        filesystem: tokens[..n - 5].join(" "),
        total: kb(tokens[n - 5])?,
        used: kb(tokens[n - 4])?,
        available: kb(tokens[n - 3])?,
    })
}

/// busybox `ps` abbreviates large sizes (`1234m`, `2g`); procps prints plain kB.
fn parse_kb(value: &str) -> Option<u64> {
    let (digits, multiplier) = match value.chars().last()? {
        'k' | 'K' => (&value[..value.len() - 1], 1),
        'm' | 'M' => (&value[..value.len() - 1], 1024),
        'g' | 'G' => (&value[..value.len() - 1], 1024 * 1024),
        _ => (value, 1),
    };
    Some(digits.parse::<u64>().ok()? * multiplier * 1024)
}

/// Takes `n` whitespace-separated fields and returns them plus the untouched remainder
/// (the command line, whose inner spacing must be preserved).
fn split_fields(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut fields = Vec::with_capacity(n);
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((fields, rest.trim_end()))
}

/// Parses `ps` output in either flavor. Columns:
/// - procps:  `PID PPID USER %CPU RSS COMMAND`
/// - busybox: `PID PPID USER RSS COMMAND`
///
/// Rows belonging to the probe itself (`self_pid` and its children) are dropped.
pub fn parse_ps(lines: &[&str], procps: bool, self_pid: Option<u32>) -> Vec<LinuxProcess> {
    let n = if procps { 5 } else { 4 };
    let mut processes: Vec<LinuxProcess> = lines
        .iter()
        .skip(1)
        .filter_map(|line| {
            let (f, command) = split_fields(line, n)?;
            let (cpu, rss) = if procps {
                (f[3].parse::<f32>().ok(), f[4])
            } else {
                (None, f[3])
            };
            Some(LinuxProcess {
                pid: f[0].parse().ok()?,
                ppid: f[1].parse().ok()?,
                user: f[2].to_string(),
                cpu,
                rss: parse_kb(rss)?,
                command: command.to_string(),
            })
        })
        .filter(|p| self_pid.is_none_or(|s| p.pid != s && p.ppid != s))
        .collect();

    processes.sort_by(|a, b| {
        b.rss
            .cmp(&a.rss)
            .then_with(|| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)))
    });
    processes.truncate(PROCESS_LIMIT);
    processes
}

struct GuestSample {
    memory: GuestMemory,
    load: LoadAverage,
    cpu_times: Option<CpuTimes>,
    cpu_count: u32,
    disk: Option<DiskUsage>,
    processes: Vec<LinuxProcess>,
}

fn parse_guest(text: &str) -> Result<GuestSample, String> {
    let sections = split_sections(text);
    let lines = |name: &str| sections.get(name).map(|s| s.lines.as_slice()).unwrap_or(&[]);

    let memory = parse_meminfo(lines("meminfo"));
    if memory.total == 0 {
        return Err("Não foi possível ler /proc/meminfo da distro".into());
    }
    let ps = sections.get("ps");
    Ok(GuestSample {
        memory,
        load: lines("loadavg")
            .first()
            .and_then(|l| parse_loadavg(l))
            .unwrap_or_default(),
        cpu_times: lines("stat").first().and_then(|l| parse_cpu_times(l)),
        cpu_count: lines("nproc")
            .first()
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or(1),
        disk: parse_df(lines("df")),
        processes: ps
            .map(|s| {
                let self_pid = sections.get("self").and_then(|s| s.arg.parse().ok());
                parse_ps(&s.lines, s.arg == "procps", self_pid)
            })
            .unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Guest (Linux) — collection
// ---------------------------------------------------------------------------

async fn sample_guest(distro: &str, sampler: &GuestSampler) -> Result<GuestMetrics, String> {
    let text = run(sh_in_distro(distro, None, GUEST_SCRIPT, &[]), GUEST_TIMEOUT).await?;
    let sample = parse_guest(&text)?;

    let cpu_usage = sample.cpu_times.and_then(|now| {
        let mut previous = sampler.0.lock().unwrap_or_else(|e| e.into_inner());
        previous
            .insert(distro.to_string(), now)
            .and_then(|prev| cpu_usage_between(prev, now))
    });

    Ok(GuestMetrics {
        memory: sample.memory,
        load: sample.load,
        cpu_usage,
        cpu_count: sample.cpu_count,
        disk: sample.disk,
        processes: sample.processes,
    })
}

pub(crate) async fn collect_metrics(
    distro: &str,
    host: &HostMonitor,
    sampler: &GuestSampler,
) -> Result<WslResourceMetrics, String> {
    validate_distro_name(distro)?;

    let guest = async {
        if is_running(distro).await? {
            sample_guest(distro, sampler).await.map(Some)
        } else {
            Ok(None)
        }
    };
    let (host, guest) = tokio::join!(host.sample(), guest);

    let (guest, guest_error) = match guest {
        Ok(guest) => (guest, None),
        Err(e) => (None, Some(e)),
    };
    Ok(WslResourceMetrics {
        distro: distro.to_string(),
        host: host?,
        guest,
        guest_error,
    })
}

const DROP_CACHES_SCRIPT: &str = "echo @@before; cat /proc/meminfo
sync && echo 3 > /proc/sys/vm/drop_caches || exit 1
echo @@after; cat /proc/meminfo";

pub(crate) async fn drop_caches(distro: &str) -> Result<DropCachesResult, String> {
    ensure_running(distro).await?;
    let cmd = sh_in_distro(distro, Some("root"), DROP_CACHES_SCRIPT, &[]);
    let text = run(cmd, GUEST_TIMEOUT).await?;
    let sections = split_sections(&text);
    let mem = |name: &str| sections.get(name).map(|s| parse_meminfo(&s.lines));
    let (Some(before), Some(after)) = (mem("before"), mem("after")) else {
        return Err("Não foi possível confirmar a limpeza do cache".into());
    };
    Ok(DropCachesResult {
        cache_before: before.cache,
        cache_after: after.cache,
        free_before: before.free,
        free_after: after.free,
    })
}

pub(crate) fn validate_signal(signal: Option<&str>) -> Result<&'static str, String> {
    match signal.map(str::to_ascii_uppercase).as_deref() {
        None | Some("TERM") | Some("SIGTERM") => Ok("TERM"),
        Some("KILL") | Some("SIGKILL") => Ok("KILL"),
        Some(other) => Err(format!("Sinal não permitido: {other} (use TERM ou KILL)")),
    }
}

pub(crate) async fn kill_process(distro: &str, pid: u32, signal: Option<&str>) -> Result<(), String> {
    // PID 1 is the distro's init; killing it takes the whole distro down.
    if pid <= 1 {
        return Err(format!("PID inválido: {pid}"));
    }
    let signal = validate_signal(signal)?;
    ensure_running(distro).await?;
    let pid = pid.to_string();
    let cmd = sh_in_distro(distro, Some("root"), "kill -s $1 $2", &[signal, &pid]);
    run(cmd, GUEST_TIMEOUT).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_wsl_resource_metrics(
    distro: String,
    host: tauri::State<'_, HostMonitor>,
    sampler: tauri::State<'_, GuestSampler>,
) -> Result<WslResourceMetrics, String> {
    collect_metrics(&distro, &host, &sampler).await
}

#[tauri::command]
pub async fn free_wsl_cached_memory(distro: String) -> Result<DropCachesResult, String> {
    drop_caches(&distro).await
}

#[tauri::command]
pub async fn kill_linux_process(distro: String, pid: u32, signal: Option<String>) -> Result<(), String> {
    kill_process(&distro, pid, signal.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:        8000000 kB
MemFree:         2000000 kB
MemAvailable:    5000000 kB
Buffers:          100000 kB
Cached:          2500000 kB
SwapCached:            0 kB
SReclaimable:     400000 kB";

    #[test]
    fn parses_meminfo() {
        let lines: Vec<&str> = MEMINFO.lines().collect();
        let m = parse_meminfo(&lines);
        assert_eq!(m.total, 8_000_000 * 1024);
        assert_eq!(m.free, 2_000_000 * 1024);
        assert_eq!(m.available, 5_000_000 * 1024);
        assert_eq!(m.cache, 3_000_000 * 1024);
        assert_eq!(m.used, 3_000_000 * 1024);
    }

    #[test]
    fn meminfo_without_memavailable() {
        let m = parse_meminfo(&["MemTotal: 1000 kB", "MemFree: 200 kB", "Cached: 300 kB"]);
        assert_eq!(m.available, 500 * 1024);
        assert_eq!(m.used, 500 * 1024);
    }

    #[test]
    fn parses_loadavg() {
        let l = parse_loadavg("0.52 0.58 1.59 2/1234 5678").unwrap();
        assert_eq!((l.one, l.five, l.fifteen), (0.52, 0.58, 1.59));
        assert_eq!((l.running, l.total), (2, 1234));
        assert!(parse_loadavg("garbage").is_none());
    }

    #[test]
    fn computes_cpu_delta() {
        let a = parse_cpu_times("cpu  100 0 100 800 0 0 0 0 0 0").unwrap();
        let b = parse_cpu_times("cpu  150 0 150 900 0 0 0 0 0 0").unwrap();
        assert_eq!(cpu_usage_between(a, b), Some(50.0));
        assert!(parse_cpu_times("cpu0 1 2 3 4").is_none());
    }

    #[test]
    fn parses_df_both_flavors() {
        let procps = [
            "Filesystem     1024-blocks     Used Available Capacity Mounted on",
            "/dev/sdc        1055762868 78307964 923751432       8% /",
        ];
        let d = parse_df(&procps).unwrap();
        assert_eq!(d.filesystem, "/dev/sdc");
        assert_eq!(d.total, 1_055_762_868 * 1024);
        assert_eq!(d.used, 78_307_964 * 1024);
        assert_eq!(d.available, 923_751_432 * 1024);

        let busybox = [
            "Filesystem           1024-blocks    Used Available Capacity Mounted on",
            "/dev/sdd                138297     72229     55316  57% /",
        ];
        assert_eq!(parse_df(&busybox).unwrap().used, 72_229 * 1024);
        assert!(parse_df(&["Filesystem"]).is_none());
    }

    #[test]
    fn parses_procps_ps() {
        let lines = [
            "    PID    PPID USER     %CPU   RSS COMMAND",
            "      1       0 root      0.0 12788 /sbin/init",
            "    420       1 kyo      12.5 204800 node  server.js --port 3000",
            "    900     899 kyo       0.0  3000 ps -ww -eo pid,ppid,user,pcpu,rss,args",
            "    899       1 kyo       0.0  1000 sh -c ...",
        ];
        let ps = parse_ps(&lines, true, Some(899));
        assert_eq!(ps.len(), 2, "probe shell and its ps must be filtered out");
        assert_eq!(ps[0].pid, 420);
        assert_eq!(ps[0].cpu, Some(12.5));
        assert_eq!(ps[0].rss, 204_800 * 1024);
        assert_eq!(ps[0].command, "node  server.js --port 3000");
        assert_eq!(ps[1].command, "/sbin/init");
    }

    #[test]
    fn parses_busybox_ps() {
        let lines = [
            "PID   PPID  USER     RSS  COMMAND",
            "    1     0 root     1756 {init(docker-des} /init",
            "   42     1 root     12m  /usr/bin/dockerd",
            "   43     1 root     2g   containerd",
        ];
        let ps = parse_ps(&lines, false, None);
        assert_eq!(ps.iter().map(|p| p.pid).collect::<Vec<_>>(), [43, 42, 1]);
        assert_eq!(ps[0].rss, 2 * 1024 * 1024 * 1024);
        assert_eq!(ps[1].rss, 12 * 1024 * 1024);
        assert_eq!(ps[2].cpu, None);
        assert_eq!(ps[2].command, "{init(docker-des} /init");
    }

    #[test]
    fn parses_full_guest_output() {
        let text = format!(
            "@@meminfo\n{MEMINFO}\n@@loadavg\n0.10 0.20 0.30 1/50 99\n@@stat\ncpu  1 2 3 4 5 6 7 8 0 0\n\
             @@nproc\n12\n@@df\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sdc 100 40 60 40% /\n\
             @@self 77\n@@ps busybox\nPID PPID USER RSS COMMAND\n1 0 root 100 /init\n77 1 root 50 sh\n"
        );
        let g = parse_guest(&text).unwrap();
        assert_eq!(g.cpu_count, 12);
        assert_eq!(g.load.total, 50);
        assert_eq!(g.disk.unwrap().total, 100 * 1024);
        assert_eq!(g.processes.len(), 1);
        assert!(parse_guest("@@meminfo\n").is_err());
    }

    #[test]
    fn validates_signals() {
        assert_eq!(validate_signal(None), Ok("TERM"));
        assert_eq!(validate_signal(Some("kill")), Ok("KILL"));
        assert_eq!(validate_signal(Some("SIGTERM")), Ok("TERM"));
        assert!(validate_signal(Some("HUP")).is_err());
        assert!(validate_signal(Some("9; rm -rf /")).is_err());
    }

    fn entry(pid: u32, image: &str, ws: u64, private: u64) -> nt::ProcessEntry {
        nt::ProcessEntry {
            pid,
            display_name: image.into(),
            name: nt::normalize_name(image),
            working_set: ws,
            private_working_set: private,
            cpu_time: 0,
        }
    }

    #[test]
    fn sums_wsl_processes() {
        // Regression: the VM must win even when it shows up after the helpers, and helpers
        // must never be mistaken for the VM.
        let entries = vec![
            entry(10, "wslservice.exe", 40, 20),
            entry(11, "wslhost.exe", 30, 10),
            entry(12, "wsl.exe", 8, 0),
            entry(13, "chrome.exe", 999, 999),
            entry(14, "vmmemWSL", 2000, 1000),
            entry(15, "vmmem", 500, 500),
        ];
        let (vm, helpers) = summarize_wsl(&entries);
        let vm = vm.expect("vm");
        assert_eq!((vm.pid, vm.display_name.as_str(), vm.memory()), (14, "vmmemWSL", 1000));
        assert_eq!(helpers, WslHelpers { memory: 20 + 10 + 8, processes: 3 });

        // Older WSL: plain vmmem. VM off: helpers only.
        assert_eq!(summarize_wsl(&entries[4..][1..]).0.map(|e| e.pid), Some(15));
        let (vm, helpers) = summarize_wsl(&entries[..3]);
        assert!(vm.is_none());
        assert_eq!(helpers.processes, 3);
    }

    #[test]
    fn live_host_snapshot_lists_processes() {
        // Not ignored: only reads this machine's process list.
        let entries = nt::snapshot();
        assert!(entries.len() > 10);
        assert!(entries.iter().any(|e| e.name == "explorer.exe" || e.name == "system.exe"));
        let (vm, helpers) = summarize_wsl(&entries);
        println!("vm={:?} helpers={helpers:?}", vm.map(|e| (&e.display_name, e.working_set, e.private_working_set)));
    }

    // Live tests against the real WSL/Windows. Run with:
    //   cargo test -- --ignored --nocapture --test-threads=1 live_
    // Set WSL_TEST_DISTRO to a running distro.

    #[tokio::test]
    #[ignore]
    async fn live_metrics() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let (host, sampler) = (HostMonitor::default(), GuestSampler::default());
        collect_metrics(&distro, &host, &sampler).await.expect("first sample");
        tokio::time::sleep(Duration::from_secs(2)).await;
        let m = collect_metrics(&distro, &host, &sampler).await.expect("second sample");
        println!("{:#?}", m.host);
        if let Some(g) = &m.guest {
            println!("{:#?}\n{:#?}\n{:#?}\ncpu={:?} ncpu={}", g.memory, g.load, g.disk, g.cpu_usage, g.cpu_count);
            for p in g.processes.iter().take(8) {
                println!("{:>7} {:<10} {:>6?} {:>10} {}", p.pid, p.user, p.cpu, p.rss / 1024, p.command);
            }
        }
        assert!(m.host.total_memory > 0);
        assert!(m.guest_error.is_none(), "{:?}", m.guest_error);
    }

    #[tokio::test]
    #[ignore]
    async fn live_drop_caches() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let r = drop_caches(&distro).await.expect("drop_caches");
        println!("{r:#?}");
        assert!(r.cache_after <= r.cache_before);
    }

    #[tokio::test]
    #[ignore]
    async fn live_kill_process() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        // A victim process inside the distro, found back through the ps parser.
        let mut victim = crate::wsl::wsl_command();
        victim.args(["-d", &distro, "--exec", "sleep", "4242"]);
        let _child = victim.spawn().expect("spawn victim");
        tokio::time::sleep(Duration::from_secs(2)).await;

        let find = || async {
            let text = run(sh_in_distro(&distro, None, GUEST_SCRIPT, &[]), GUEST_TIMEOUT).await.unwrap();
            let sections = split_sections(&text);
            let ps = &sections["ps"];
            // Unlimited list: the victim is tiny and would be cut by PROCESS_LIMIT.
            ps.lines
                .iter()
                .skip(1)
                .filter_map(|l| split_fields(l, if ps.arg == "procps" { 5 } else { 4 }))
                .find(|(_, cmd)| *cmd == "sleep 4242")
                .and_then(|(f, _)| f[0].parse::<u32>().ok())
        };
        let pid = find().await.expect("victim visible in ps");
        println!("victim pid {pid}");

        assert!(kill_process(&distro, 1, None).await.is_err(), "PID 1 must be refused");
        assert!(kill_process(&distro, pid, Some("HUP")).await.is_err());
        kill_process(&distro, pid, Some("TERM")).await.expect("kill TERM");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(find().await, None, "victim still alive");
    }
}
