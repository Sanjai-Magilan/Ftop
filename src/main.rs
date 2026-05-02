use crossterm::cursor::{Hide, MoveTo, SetCursorStyle, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use libc::{kill, SIGKILL};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, stdout, Stdout, Write};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{CpuExt, PidExt, ProcessExt, ProcessStatus, System, SystemExt};

const ASSUMED_CPU_PACKAGE_POWER_W: f32 = 65.0;
const MIN_DISPLAY_POWER_W: f32 = 0.1;
const NET_EMA_ALPHA: f64 = 0.35;
const MIN_NET_DT: f64 = 0.25;
const UI_POLL_MS: u64 = 50;
const COLOR_APP_BG: Color = Color::Rgb {
    r: 13,
    g: 17,
    b: 23,
};
const COLOR_PANEL_BG: Color = Color::Rgb {
    r: 17,
    g: 24,
    b: 39,
};
const COLOR_TEXT: Color = Color::Rgb {
    r: 226,
    g: 232,
    b: 240,
};
const COLOR_MUTED: Color = Color::Rgb {
    r: 148,
    g: 163,
    b: 184,
};
const COLOR_CYAN: Color = Color::Rgb {
    r: 56,
    g: 189,
    b: 248,
};
const COLOR_PURPLE: Color = Color::Rgb {
    r: 167,
    g: 139,
    b: 250,
};
const COLOR_GREEN: Color = Color::Rgb {
    r: 52,
    g: 211,
    b: 153,
};
const COLOR_YELLOW: Color = Color::Rgb {
    r: 251,
    g: 191,
    b: 36,
};
const COLOR_RED: Color = Color::Rgb {
    r: 251,
    g: 113,
    b: 133,
};
const COLOR_SELECT_BG: Color = Color::Rgb {
    r: 37,
    g: 99,
    b: 235,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Cpu,
    Mem,
    Power,
}

#[derive(Clone, Copy)]
enum ProcessTableMode {
    Full,
    Compact,
    Tiny,
}

struct ProcRow {
    pid: i32,
    cpu_percent: f32,
    mem_percent: f32,
    power_watts: Option<f32>,
    uptime_secs: u64,
    shared_bytes: u64,
    virtual_bytes: u64,
    resident_bytes: u64,
    priority: i64,
    name: String,
}

struct MemInfo {
    total: u64,
    free: u64,
    buffers: u64,
    cached: u64,
    sreclaimable: u64,
    shmem: u64,
    available: u64,
}

struct RuntimeMetrics {
    now_text: String,
    host: String,
    cpu_total: f32,
    cpu_per_core: Vec<f32>,
    load_1: f64,
    load_5: f64,
    load_15: f64,
    uptime_text: String,
    mem_used: u64,
    mem_total: u64,
    mem_percent: f32,
    swap_used: u64,
    swap_total: u64,
    swap_percent: f32,
    disk_used: u64,
    disk_total: u64,
    disk_percent: f32,
    net_total_down: u64,
    net_total_up: u64,
    net_rate_down: f64,
    net_rate_up: f64,
    proc_total: usize,
    procs: Vec<ProcRow>,
}

struct AppState {
    sort_key: SortKey,
    scroll_offset: usize,
    selected_index: usize,
    locked_pid: Option<i32>,
    search_mode: bool,
    search_query: String,
    search_input: String,
    status_message: String,
    status_until: Instant,
    force_refresh: bool,
    down_rate_ema: f64,
    up_rate_ema: f64,
}

#[derive(Clone, Copy)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sort_key: SortKey::Cpu,
            scroll_offset: 0,
            selected_index: 0,
            locked_pid: None,
            search_mode: false,
            search_query: String::new(),
            search_input: String::new(),
            status_message: String::new(),
            status_until: Instant::now(),
            force_refresh: true,
            down_rate_ema: 0.0,
            up_rate_ema: 0.0,
        }
    }
}

fn human_bytes(mut value: f64, decimals: usize) -> String {
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut i = 0;
    while value >= 1024.0 && i < units.len() - 1 {
        value /= 1024.0;
        i += 1;
    }
    format!("{:.*}{}", decimals, value, units[i])
}

fn human_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let rem1 = seconds % 86400;
    let hours = rem1 / 3600;
    let rem2 = rem1 % 3600;
    let minutes = rem2 / 60;
    let secs = rem2 % 60;
    if days > 0 {
        format!("{}d {:02}:{:02}:{:02}", days, hours, minutes, secs)
    } else {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    }
}

fn format_process_memory(bytes: u64, width: usize) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    let value = if mb >= 1000.0 {
        format!("{:.1}GB", mb / 1000.0)
    } else {
        format!("{:.1}MB", mb)
    };
    format!("{:>width$}", value, width = width)
}

fn process_table_mode(width: usize) -> ProcessTableMode {
    if width >= 96 {
        ProcessTableMode::Full
    } else if width >= 78 {
        ProcessTableMode::Compact
    } else {
        ProcessTableMode::Tiny
    }
}

fn process_table_fixed_width(mode: ProcessTableMode) -> usize {
    match mode {
        ProcessTableMode::Full => 78,
        ProcessTableMode::Compact => 60,
        ProcessTableMode::Tiny => 33,
    }
}

fn process_table_header(mode: ProcessTableMode) -> String {
    match mode {
        ProcessTableMode::Full => format!(
            "{:>8} {:>5}  {:>5}  {:>4}  {:>11} {:>8} {:>9} {:>8}  {:>6}  {}",
            "PID", "CPU%", "MEM%", "PRI", "UPTIME", "SHR", "VIRT", "RES", "PWR", "NAME"
        ),
        ProcessTableMode::Compact => format!(
            "{:>8} {:>5}  {:>5}  {:>8} {:>9} {:>8}  {:>6}  {}",
            "PID", "CPU%", "MEM%", "SHR", "VIRT", "RES", "PWR", "NAME"
        ),
        ProcessTableMode::Tiny => format!(
            "{:>8} {:>5}  {:>5}  {:>8}  {}",
            "PID", "CPU%", "MEM%", "RES", "NAME"
        ),
    }
}

fn process_table_row(p: &ProcRow, mode: ProcessTableMode, width: usize) -> String {
    let fixed_width = process_table_fixed_width(mode);
    let name_w = width.saturating_sub(fixed_width).max(1);
    let name = truncate_to_width(&p.name, name_w);

    match mode {
        ProcessTableMode::Full => format!(
            "{:>8} {:>5.1}  {:>5.1}  {:>4}  {:>11} {:>8} {:>9} {:>8}  {:>6}  {}",
            p.pid,
            p.cpu_percent,
            p.mem_percent,
            p.priority,
            human_uptime(p.uptime_secs),
            format_process_memory(p.shared_bytes, 8),
            format_process_memory(p.virtual_bytes, 9),
            format_process_memory(p.resident_bytes, 8),
            format_power_usage(p.power_watts),
            name
        ),
        ProcessTableMode::Compact => format!(
            "{:>8} {:>5.1}  {:>5.1}  {:>8} {:>9} {:>8}  {:>6}  {}",
            p.pid,
            p.cpu_percent,
            p.mem_percent,
            format_process_memory(p.shared_bytes, 8),
            format_process_memory(p.virtual_bytes, 9),
            format_process_memory(p.resident_bytes, 8),
            format_power_usage(p.power_watts),
            name
        ),
        ProcessTableMode::Tiny => format!(
            "{:>8} {:>5.1}  {:>5.1}  {:>8}  {}",
            p.pid,
            p.cpu_percent,
            p.mem_percent,
            format_process_memory(p.resident_bytes, 8),
            name
        ),
    }
}

fn progress_bar(percent: f32, width: usize) -> String {
    let width = max(10, width);
    let fill = ((percent / 100.0) * width as f32).round() as isize;
    let fill = min(width as isize, max(0, fill)) as usize;
    format!("{}{}", "█".repeat(fill), "░".repeat(width - fill))
}

fn estimate_process_power_watts(cpu_percent: f32, logical_cpus: usize) -> Option<f32> {
    if logical_cpus == 0 || cpu_percent <= 0.0 {
        return None;
    }
    let usage_fraction = cpu_percent / (100.0 * logical_cpus as f32);
    let watts = (usage_fraction * ASSUMED_CPU_PACKAGE_POWER_W).max(0.0);
    if watts < MIN_DISPLAY_POWER_W {
        None
    } else {
        Some(watts)
    }
}

fn format_power_usage(watts: Option<f32>) -> String {
    match watts {
        Some(v) => format!("{:.1}W", v),
        None => "-".to_string(),
    }
}

fn page_size_bytes() -> u64 {
    let page_sz = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_sz <= 0 {
        4096
    } else {
        page_sz as u64
    }
}

fn read_shared_bytes(pid: i32, page_size: u64) -> u64 {
    let path = format!("/proc/{}/statm", pid);
    let Ok(content) = fs::read_to_string(path) else {
        return 0;
    };
    let parts = content.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 3 {
        return 0;
    }
    let Ok(shared_pages) = parts[2].parse::<u64>() else {
        return 0;
    };
    shared_pages.saturating_mul(page_size)
}

fn read_process_priority(pid: i32) -> i64 {
    let path = format!("/proc/{}/stat", pid);
    let Ok(content) = fs::read_to_string(path) else {
        return 0;
    };

    let Some(end_comm) = content.rfind(") ") else {
        return 0;
    };

    let tail = &content[end_comm + 2..];
    let fields = tail.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 16 {
        return 0;
    }

    fields[15].parse::<i64>().unwrap_or(0)
}

fn now_text() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

fn parse_meminfo() -> Option<MemInfo> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = 0_u64;
    let mut free = 0_u64;
    let mut buffers = 0_u64;
    let mut cached = 0_u64;
    let mut sreclaimable = 0_u64;
    let mut shmem = 0_u64;
    let mut available = 0_u64;

    for line in content.lines() {
        let mut parts = line.split(':');
        let Some(key) = parts.next().map(|v| v.trim()) else {
            continue;
        };
        let Some(val_part) = parts.next().map(|v| v.trim()) else {
            continue;
        };
        let Some(kb_str) = val_part.split_whitespace().next() else {
            continue;
        };
        let Ok(kb) = kb_str.parse::<u64>() else {
            continue;
        };
        let bytes = kb.saturating_mul(1024);
        match key {
            "MemTotal" => total = bytes,
            "MemFree" => free = bytes,
            "Buffers" => buffers = bytes,
            "Cached" => cached = bytes,
            "SReclaimable" => sreclaimable = bytes,
            "Shmem" => shmem = bytes,
            "MemAvailable" => available = bytes,
            _ => {}
        }
    }

    if total == 0 {
        None
    } else {
        Some(MemInfo {
            total,
            free,
            buffers,
            cached,
            sreclaimable,
            shmem,
            available,
        })
    }
}

fn sample_memory_usage(system: &System) -> (u64, u64, f32) {
    if let Some(mi) = parse_meminfo() {
        // Linux "actual used" approximation:
        // used = total - free - buffers - (cached + reclaimable - shmem)
        let effective_cache = mi
            .cached
            .saturating_add(mi.sreclaimable)
            .saturating_sub(mi.shmem);
        let mut used = mi.total.saturating_sub(
            mi.free
                .saturating_add(mi.buffers)
                .saturating_add(effective_cache),
        );

        // Fallback when formula is unstable on some kernels/containers.
        if mi.available > 0 && (used == 0 || used > mi.total) {
            used = mi.total.saturating_sub(mi.available);
        }

        let pct = if mi.total > 0 {
            (used as f64 / mi.total as f64 * 100.0) as f32
        } else {
            0.0
        };
        return (used, mi.total, pct);
    }

    let total = system.total_memory();
    let used = system.used_memory();
    let pct = if total > 0 {
        (used as f64 / total as f64 * 100.0) as f32
    } else {
        0.0
    };
    (used, total, pct)
}

fn read_net_totals() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/net/dev").ok()?;
    let mut recv_total = 0_u64;
    let mut sent_total = 0_u64;

    for line in content.lines().skip(2) {
        let line = line.trim();
        if line.is_empty() || !line.contains(':') {
            continue;
        }

        let mut parts = line.split(':');
        let Some(iface) = parts.next().map(|v| v.trim()) else {
            continue;
        };
        let Some(stats_part) = parts.next() else {
            continue;
        };
        let stats = stats_part.split_whitespace().collect::<Vec<_>>();
        if iface == "lo" || stats.len() < 16 {
            continue;
        }

        let Ok(recv) = stats[0].parse::<u64>() else {
            continue;
        };
        let Ok(sent) = stats[8].parse::<u64>() else {
            continue;
        };
        recv_total = recv_total.saturating_add(recv);
        sent_total = sent_total.saturating_add(sent);
    }

    Some((recv_total, sent_total))
}

fn filter_processes(mut procs: Vec<ProcRow>, query: &str) -> Vec<ProcRow> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return procs;
    }

    procs.retain(|p| p.pid.to_string().contains(&q) || p.name.to_lowercase().contains(&q));
    procs
}

fn read_ppid_map() -> HashMap<i32, Vec<i32>> {
    let mut map: HashMap<i32, Vec<i32>> = HashMap::new();

    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(pid) = file_name.parse::<i32>() else {
                continue;
            };

            let stat_path = format!("/proc/{}/stat", pid);
            let Ok(stat) = fs::read_to_string(stat_path) else {
                continue;
            };
            let fields = stat.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5 {
                continue;
            }
            let Ok(ppid) = fields[3].parse::<i32>() else {
                continue;
            };

            map.entry(ppid).or_default().push(pid);
        }
    }

    map
}

fn collect_descendants(pid: i32, map: &HashMap<i32, Vec<i32>>, out: &mut Vec<i32>) {
    if let Some(children) = map.get(&pid) {
        for child in children {
            collect_descendants(*child, map, out);
            out.push(*child);
        }
    }
}

fn kill_process_tree(pid: i32) -> (bool, String) {
    if pid <= 0 {
        return (false, "Invalid process id".to_string());
    }

    let ppid_map = read_ppid_map();
    let mut targets = Vec::new();
    collect_descendants(pid, &ppid_map, &mut targets);
    targets.push(pid);

    let mut killed = 0_u32;
    let mut denied = 0_u32;

    for target in targets {
        let rc = unsafe { kill(target, SIGKILL) };
        if rc == 0 {
            killed += 1;
        } else {
            let err = io::Error::last_os_error();
            if let Some(code) = err.raw_os_error() {
                if code == libc::EPERM {
                    denied += 1;
                }
            }
        }
    }

    if killed == 0 {
        if denied > 0 {
            return (false, format!("Permission denied for PID {}", pid));
        }
        return (true, format!("PID {} no longer exists", pid));
    }

    if denied > 0 {
        return (
            false,
            format!(
                "Killed {} proc(s), denied {} for PID {}",
                killed, denied, pid
            ),
        );
    }

    (
        true,
        format!(
            "Killed PID {} (and {} child proc(s))",
            pid,
            killed.saturating_sub(1)
        ),
    )
}

fn parse_args() -> (f64, usize) {
    let mut refresh = 1.0_f64;
    let mut top = 0_usize;

    let args = env::args().collect::<Vec<_>>();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-r" | "--refresh" => {
                if i + 1 < args.len() {
                    if let Ok(v) = args[i + 1].parse::<f64>() {
                        refresh = v;
                    }
                    i += 1;
                }
            }
            "-n" | "--top" => {
                if i + 1 < args.len() {
                    if let Ok(v) = args[i + 1].parse::<usize>() {
                        top = v;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    (refresh.max(0.2), top)
}

fn truncate_to_width(s: &str, width: usize) -> String {
    s.chars().take(width).collect::<String>()
}

fn fit_to_width(text: &str, width: usize) -> String {
    let shown = if width == 0 {
        String::new()
    } else {
        truncate_to_width(text, width)
    };
    let shown_width = shown.chars().count();
    if shown_width < width {
        format!("{}{}", shown, " ".repeat(width - shown_width))
    } else {
        shown
    }
}

fn usage_color(percent: f32) -> Color {
    if percent >= 85.0 {
        COLOR_RED
    } else if percent >= 65.0 {
        COLOR_YELLOW
    } else {
        COLOR_GREEN
    }
}

fn draw_segment(
    stdout: &mut Stdout,
    x: u16,
    y: u16,
    text: &str,
    width: u16,
    fg: Option<Color>,
    bg: Option<Color>,
    reverse: bool,
    bold: bool,
) -> io::Result<()> {
    if width == 0 {
        return Ok(());
    }

    queue!(stdout, MoveTo(x, y))?;
    if let Some(color) = fg {
        queue!(stdout, SetForegroundColor(color))?;
    }
    if let Some(color) = bg {
        queue!(stdout, SetBackgroundColor(color))?;
    }
    if reverse {
        queue!(stdout, SetAttribute(Attribute::Reverse))?;
    }
    if bold {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }

    queue!(stdout, Print(fit_to_width(text, width as usize)))?;
    queue!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
    Ok(())
}

fn draw_box(stdout: &mut Stdout, rect: Rect, title: &str, color: Color) -> io::Result<()> {
    if rect.w < 2 || rect.h < 2 {
        return Ok(());
    }

    let inner_w = rect.w.saturating_sub(2) as usize;
    let title_text = if title.is_empty() {
        String::new()
    } else {
        format!(" {} ", title)
    };
    let title_width = title_text.chars().count();
    let top_middle = if title_width >= inner_w {
        truncate_to_width(&title_text, inner_w)
    } else {
        format!("{}{}", title_text, "─".repeat(inner_w - title_width))
    };
    draw_segment(
        stdout,
        rect.x,
        rect.y,
        &format!("╭{}╮", top_middle),
        rect.w,
        Some(color),
        Some(COLOR_PANEL_BG),
        false,
        true,
    )?;

    for y in rect.y + 1..rect.y + rect.h.saturating_sub(1) {
        draw_segment(
            stdout,
            rect.x,
            y,
            &format!("│{}│", " ".repeat(inner_w)),
            rect.w,
            Some(COLOR_MUTED),
            Some(COLOR_PANEL_BG),
            false,
            false,
        )?;
    }

    draw_segment(
        stdout,
        rect.x,
        rect.y + rect.h.saturating_sub(1),
        &format!("╰{}╯", "─".repeat(inner_w)),
        rect.w,
        Some(color),
        Some(COLOR_PANEL_BG),
        false,
        true,
    )
}

fn draw_panel_line(
    stdout: &mut Stdout,
    rect: Rect,
    row: u16,
    text: &str,
    fg: Option<Color>,
    reverse: bool,
    bold: bool,
) -> io::Result<()> {
    if rect.w <= 2 || rect.h <= 2 || row >= rect.h - 2 {
        return Ok(());
    }
    draw_segment(
        stdout,
        rect.x + 1,
        rect.y + 1 + row,
        text,
        rect.w - 2,
        fg,
        Some(COLOR_PANEL_BG),
        reverse,
        bold,
    )
}

fn draw_meter(
    stdout: &mut Stdout,
    rect: Rect,
    row: u16,
    label: &str,
    percent: f32,
    detail: &str,
) -> io::Result<()> {
    let inner_w = rect.w.saturating_sub(2) as usize;
    let fixed_w = label.chars().count() + detail.chars().count() + 13;
    let bar_w = max(8, inner_w.saturating_sub(fixed_w));
    let line = format!(
        "{:<7} {:>5.1}% [{}] {}",
        label,
        percent,
        progress_bar(percent, bar_w),
        detail
    );
    draw_panel_line(
        stdout,
        rect,
        row,
        &line,
        Some(usage_color(percent)),
        false,
        false,
    )
}

fn sample_disk_usage() -> (u64, u64) {
    // Try to read /proc/mounts for the root filesystem
    if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "/" {
                // Found root mount, try statfs on it
                if let Ok(statvfs_result) = unsafe {
                    let mut stat: libc::statvfs = std::mem::zeroed();
                    let path = std::ffi::CString::new("/").unwrap();
                    let rc = libc::statvfs(path.as_ptr(), &mut stat);
                    if rc == 0 {
                        Ok(stat)
                    } else {
                        Err(io::Error::last_os_error())
                    }
                } {
                    let total = statvfs_result
                        .f_blocks
                        .saturating_mul(statvfs_result.f_frsize as u64);
                    let available = statvfs_result
                        .f_bavail
                        .saturating_mul(statvfs_result.f_frsize as u64);
                    let used = total.saturating_sub(available);
                    return (total, used);
                }
            }
        }
    }

    (0, 0)
}

fn build_processes(
    system: &System,
    sort_key: SortKey,
    logical_cpus: usize,
    top: usize,
) -> (Vec<ProcRow>, usize) {
    let total_mem_bytes = system.total_memory().max(1);
    let page_size = page_size_bytes();
    let mut rows = Vec::new();

    for (pid, proc_) in system.processes() {
        if proc_.status() == ProcessStatus::Zombie {
            continue;
        }

        let cpu = proc_.cpu_usage();
        let mem_pct = (proc_.memory() as f64 / total_mem_bytes as f64 * 100.0) as f32;
        let power = estimate_process_power_watts(cpu, logical_cpus);
        let pid_i32 = pid.as_u32() as i32;
        let resident_bytes = proc_.memory();
        let virtual_bytes = proc_.virtual_memory();
        let shared_bytes = read_shared_bytes(pid_i32, page_size);
        let priority = read_process_priority(pid_i32);

        rows.push(ProcRow {
            pid: pid_i32,
            cpu_percent: cpu,
            mem_percent: mem_pct,
            power_watts: power,
            uptime_secs: proc_.run_time(),
            shared_bytes,
            virtual_bytes,
            resident_bytes,
            priority,
            name: proc_.name().to_string(),
        });
    }

    match sort_key {
        SortKey::Cpu => rows.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SortKey::Mem => rows.sort_by(|a, b| {
            b.mem_percent
                .partial_cmp(&a.mem_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SortKey::Power => rows.sort_by(|a, b| {
            b.power_watts
                .unwrap_or(0.0)
                .partial_cmp(&a.power_watts.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }

    let total = rows.len();
    if top == 0 {
        (rows, total)
    } else {
        (rows.into_iter().take(top).collect(), total)
    }
}

fn sample_metrics(
    system: &mut System,
    sort_key: SortKey,
    search_query: &str,
    top: usize,
    logical_cpus: usize,
    prev_net: &mut (u64, u64),
    prev_time: &mut Instant,
    down_ema: &mut f64,
    up_ema: &mut f64,
) -> RuntimeMetrics {
    system.refresh_cpu();
    system.refresh_memory();
    system.refresh_processes();
    system.refresh_disks_list();
    system.refresh_disks();

    let host = system.host_name().unwrap_or_else(|| "unknown".to_string());
    let now_text = now_text();

    let cpu_per_core = system
        .cpus()
        .iter()
        .map(|c| c.cpu_usage())
        .collect::<Vec<_>>();
    let cpu_total = if cpu_per_core.is_empty() {
        0.0
    } else {
        cpu_per_core.iter().copied().sum::<f32>() / cpu_per_core.len() as f32
    };

    let load = system.load_average();
    let uptime_text = human_uptime(system.uptime());

    let (mem_used, mem_total, mem_percent) = sample_memory_usage(system);
    let swap_total = system.total_swap();
    let swap_used = system.used_swap();
    let swap_percent = if swap_total > 0 {
        (swap_used as f64 / swap_total as f64 * 100.0) as f32
    } else {
        0.0
    };

    let (disk_total, disk_used) = sample_disk_usage();
    let disk_percent = if disk_total > 0 {
        (disk_used as f64 / disk_total as f64 * 100.0) as f32
    } else {
        0.0
    };

    let now = Instant::now();
    let dt = (now - *prev_time).as_secs_f64().max(MIN_NET_DT);
    let (rx, tx) = read_net_totals().unwrap_or(*prev_net);
    let down_rate = rx.saturating_sub(prev_net.0) as f64 / dt;
    let up_rate = tx.saturating_sub(prev_net.1) as f64 / dt;
    *down_ema = if *down_ema <= 0.0 {
        down_rate
    } else {
        NET_EMA_ALPHA * down_rate + (1.0 - NET_EMA_ALPHA) * *down_ema
    };
    *up_ema = if *up_ema <= 0.0 {
        up_rate
    } else {
        NET_EMA_ALPHA * up_rate + (1.0 - NET_EMA_ALPHA) * *up_ema
    };
    *prev_net = (rx, tx);
    *prev_time = now;

    let (sampled, proc_total) = build_processes(system, sort_key, logical_cpus, top);
    let filtered = filter_processes(sampled, search_query);

    RuntimeMetrics {
        now_text,
        host,
        cpu_total,
        cpu_per_core,
        load_1: load.one,
        load_5: load.five,
        load_15: load.fifteen,
        uptime_text,
        mem_used,
        mem_total,
        mem_percent,
        swap_used,
        swap_total,
        swap_percent,
        disk_used,
        disk_total,
        disk_percent,
        net_total_down: rx,
        net_total_up: tx,
        net_rate_down: *down_ema,
        net_rate_up: *up_ema,
        proc_total,
        procs: filtered,
    }
}

fn run_app(refresh_rate: f64, top: usize) -> io::Result<()> {
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    terminal::enable_raw_mode()?;

    let mut system = System::new_all();
    system.refresh_all();
    thread::sleep(Duration::from_millis(120));
    system.refresh_cpu();
    system.refresh_processes();

    let logical_cpus = max(1, system.cpus().len());
    let mut prev_net = read_net_totals().unwrap_or((0, 0));
    let mut prev_time = Instant::now();

    let mut app = AppState::default();
    let mut metrics = sample_metrics(
        &mut system,
        app.sort_key,
        &app.search_query,
        top,
        logical_cpus,
        &mut prev_net,
        &mut prev_time,
        &mut app.down_rate_ema,
        &mut app.up_rate_ema,
    );

    let mut last_sample = Instant::now();

    loop {
        if event::poll(Duration::from_millis(UI_POLL_MS))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if app.search_mode {
                    match key.code {
                        KeyCode::Esc => {
                            app.search_mode = false;
                            app.search_input.clear();
                        }
                        KeyCode::Enter => {
                            app.search_query = app.search_input.trim().to_string();
                            app.search_mode = false;
                            app.search_input.clear();
                            app.selected_index = 0;
                            app.scroll_offset = 0;
                            app.force_refresh = true;
                        }
                        KeyCode::Backspace => {
                            app.search_input.pop();
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.search_input.clear();
                        }
                        KeyCode::Char(c) => {
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            {
                                app.search_input.push(c);
                            }
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => break,
                        KeyCode::Char('/') => {
                            app.search_mode = true;
                            app.search_input = app.search_query.clone();
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            app.sort_key = SortKey::Cpu;
                            app.scroll_offset = 0;
                            app.force_refresh = true;
                        }
                        KeyCode::Char('m') | KeyCode::Char('M') => {
                            app.sort_key = SortKey::Mem;
                            app.scroll_offset = 0;
                            app.force_refresh = true;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P') => {
                            app.sort_key = SortKey::Power;
                            app.scroll_offset = 0;
                            app.force_refresh = true;
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => app.force_refresh = true,
                        KeyCode::Enter => {
                            if metrics.procs.is_empty() {
                                app.status_message = "No process to lock".to_string();
                                app.status_until = Instant::now() + Duration::from_secs(2);
                            } else {
                                app.selected_index =
                                    min(app.selected_index, metrics.procs.len() - 1);
                                let sel_pid = metrics.procs[app.selected_index].pid;
                                if app.locked_pid == Some(sel_pid) {
                                    app.locked_pid = None;
                                    app.status_message = "Selection unlocked".to_string();
                                } else {
                                    app.locked_pid = Some(sel_pid);
                                    app.status_message = format!("Locked on PID {}", sel_pid);
                                }
                                app.status_until = Instant::now() + Duration::from_secs(2);
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J')
                            if app.locked_pid.is_none() =>
                        {
                            app.selected_index = app.selected_index.saturating_add(1);
                        }
                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K')
                            if app.locked_pid.is_none() =>
                        {
                            app.selected_index = app.selected_index.saturating_sub(1);
                        }
                        KeyCode::PageDown if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_add(20);
                        }
                        KeyCode::PageUp if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_sub(20);
                        }
                        KeyCode::Home if app.locked_pid.is_none() => app.selected_index = 0,
                        KeyCode::End if app.locked_pid.is_none() => {
                            app.selected_index = usize::MAX / 2
                        }
                        KeyCode::Char('X') | KeyCode::Char('x') => {
                            if metrics.procs.is_empty() {
                                app.status_message = "No process selected".to_string();
                                app.status_until = Instant::now() + Duration::from_secs(2);
                            } else {
                                app.selected_index =
                                    min(app.selected_index, metrics.procs.len() - 1);
                                let pid = metrics.procs[app.selected_index].pid;
                                let (ok, msg) = kill_process_tree(pid);
                                app.status_message = msg;
                                app.status_until = Instant::now()
                                    + if ok {
                                        Duration::from_secs(2)
                                    } else {
                                        Duration::from_millis(2800)
                                    };
                                app.force_refresh = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let metrics_interval = Duration::from_secs_f64(refresh_rate.max(0.2));
        if app.force_refresh || last_sample.elapsed() >= metrics_interval {
            metrics = sample_metrics(
                &mut system,
                app.sort_key,
                &app.search_query,
                top,
                logical_cpus,
                &mut prev_net,
                &mut prev_time,
                &mut app.down_rate_ema,
                &mut app.up_rate_ema,
            );

            if let Some(locked_pid) = app.locked_pid {
                let lock_index = metrics.procs.iter().position(|p| p.pid == locked_pid);
                if let Some(i) = lock_index {
                    app.selected_index = i;
                } else {
                    app.locked_pid = None;
                    app.status_message = format!("Locked PID {} exited", locked_pid);
                    app.status_until = Instant::now() + Duration::from_secs(2);
                }
            }

            app.force_refresh = false;
            last_sample = Instant::now();
        }

        let (w, h) = terminal::size()?;
        queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

        if h < 18 || w < 72 {
            draw_segment(
                &mut stdout,
                0,
                0,
                "Terminal too small. Resize window (min 72x18). Press q to quit.",
                w,
                Some(COLOR_RED),
                Some(COLOR_APP_BG),
                false,
                false,
            )?;
            stdout.flush()?;
            continue;
        }

        for y in 0..h {
            draw_segment(
                &mut stdout,
                0,
                y,
                "",
                w,
                Some(COLOR_TEXT),
                Some(COLOR_APP_BG),
                false,
                false,
            )?;
        }

        let top_h = if h >= 30 { 10 } else { 8 };
        let proc_y = 1 + top_h;
        let proc_h = h.saturating_sub(proc_y + 2);
        let left_w = if w >= 118 {
            max(46, (w as usize * 48 / 100) as u16)
        } else {
            max(36, w / 2)
        };
        let right_w = w.saturating_sub(left_w);
        let cpu_rect = Rect {
            x: 0,
            y: 1,
            w: left_w,
            h: top_h,
        };
        let sys_rect = Rect {
            x: left_w,
            y: 1,
            w: right_w,
            h: top_h,
        };
        let proc_rect = Rect {
            x: 0,
            y: proc_y,
            w,
            h: proc_h,
        };

        draw_segment(
            &mut stdout,
            0,
            0,
            &format!(" SysWatcher • {} • {} ", metrics.host, metrics.now_text),
            w,
            Some(COLOR_APP_BG),
            Some(COLOR_CYAN),
            false,
            true,
        )?;

        draw_box(&mut stdout, cpu_rect, "CPU", COLOR_CYAN)?;
        draw_meter(
            &mut stdout,
            cpu_rect,
            0,
            "Total",
            metrics.cpu_total,
            &format!(
                "load {:.2} {:.2} {:.2}",
                metrics.load_1, metrics.load_5, metrics.load_15
            ),
        )?;
        draw_panel_line(
            &mut stdout,
            cpu_rect,
            1,
            &format!("Uptime  {}", metrics.uptime_text),
            Some(COLOR_MUTED),
            false,
            false,
        )?;
        let core_inner = cpu_rect.w.saturating_sub(2) as usize;
        let core_cols = if core_inner >= 50 { 2 } else { 1 };
        let core_col_w = max(18, core_inner / core_cols);
        let core_bar_w = max(4, core_col_w.saturating_sub(11));
        for (idx, cpu) in metrics.cpu_per_core.iter().enumerate() {
            let row = 2 + (idx / core_cols) as u16;
            if row >= cpu_rect.h.saturating_sub(2) {
                break;
            }
            let col = idx % core_cols;
            let core_line = format!(
                "C{:02} {:>3.0}% {}",
                idx,
                cpu,
                progress_bar(*cpu, core_bar_w)
            );
            draw_segment(
                &mut stdout,
                cpu_rect.x + 1 + (col * core_col_w) as u16,
                cpu_rect.y + 1 + row,
                &core_line,
                min(core_col_w, core_inner.saturating_sub(col * core_col_w)) as u16,
                Some(usage_color(*cpu)),
                Some(COLOR_PANEL_BG),
                false,
                false,
            )?;
        }

        draw_box(&mut stdout, sys_rect, "Memory • Disk • Net", COLOR_PURPLE)?;
        draw_meter(
            &mut stdout,
            sys_rect,
            0,
            "Memory",
            metrics.mem_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.mem_used as f64, 1),
                human_bytes(metrics.mem_total as f64, 1)
            ),
        )?;
        draw_meter(
            &mut stdout,
            sys_rect,
            1,
            "Swap",
            metrics.swap_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.swap_used as f64, 1),
                human_bytes(metrics.swap_total as f64, 1)
            ),
        )?;
        draw_meter(
            &mut stdout,
            sys_rect,
            2,
            "Disk /",
            metrics.disk_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.disk_used as f64, 1),
                human_bytes(metrics.disk_total as f64, 1)
            ),
        )?;
        draw_panel_line(
            &mut stdout,
            sys_rect,
            4,
            &format!(
                "Down    {:>10}/s    Up {:>10}/s",
                human_bytes(metrics.net_rate_down, 1),
                human_bytes(metrics.net_rate_up, 1)
            ),
            Some(COLOR_GREEN),
            false,
            false,
        )?;
        draw_panel_line(
            &mut stdout,
            sys_rect,
            5,
            &format!(
                "Total ↓ {:>10}     ↑ {:>10}",
                human_bytes(metrics.net_total_down as f64, 1),
                human_bytes(metrics.net_total_up as f64, 1)
            ),
            Some(COLOR_MUTED),
            false,
            false,
        )?;

        let sort_label = match app.sort_key {
            SortKey::Cpu => "CPU",
            SortKey::Mem => "MEM",
            SortKey::Power => "PWR*",
        };

        let mut proc_header = format!(
            "Processes: {} total | Showing {} by {}",
            metrics.proc_total,
            metrics.procs.len(),
            sort_label
        );
        if !app.search_query.is_empty() {
            proc_header.push_str(&format!(" | /{}", app.search_query));
        }
        if let Some(pid) = app.locked_pid {
            proc_header.push_str(&format!(" | LOCK PID {}", pid));
        }
        draw_box(&mut stdout, proc_rect, &proc_header, COLOR_CYAN)?;
        let table_width = proc_rect.w.saturating_sub(2) as usize;
        let table_mode = process_table_mode(table_width);
        let table_header = process_table_header(table_mode);
        draw_panel_line(
            &mut stdout,
            proc_rect,
            0,
            &table_header,
            Some(COLOR_PURPLE),
            false,
            true,
        )?;

        if metrics.procs.is_empty() {
            app.selected_index = 0;
        } else {
            app.selected_index = min(app.selected_index, metrics.procs.len() - 1);
        }

        let visible_rows = max(1, proc_rect.h.saturating_sub(3) as usize);
        if app.selected_index < app.scroll_offset {
            app.scroll_offset = app.selected_index;
        } else if app.selected_index >= app.scroll_offset + visible_rows {
            app.scroll_offset = app.selected_index.saturating_sub(visible_rows - 1);
        }

        let max_scroll = metrics.procs.len().saturating_sub(visible_rows);
        app.scroll_offset = min(app.scroll_offset, max_scroll);

        let end_ix = min(metrics.procs.len(), app.scroll_offset + visible_rows);
        let visible = &metrics.procs[app.scroll_offset..end_ix];

        for (idx, p) in visible.iter().enumerate() {
            let absolute_index = app.scroll_offset + idx;
            let line = process_table_row(p, table_mode, table_width);
            let selected = absolute_index == app.selected_index;
            draw_segment(
                &mut stdout,
                proc_rect.x + 1,
                proc_rect.y + 2 + idx as u16,
                &line,
                proc_rect.w.saturating_sub(2),
                if selected {
                    Some(Color::White)
                } else {
                    Some(COLOR_TEXT)
                },
                if selected {
                    Some(COLOR_SELECT_BG)
                } else {
                    Some(COLOR_PANEL_BG)
                },
                false,
                selected,
            )?;
        }

        let mut bottom = String::new();
        if app.search_mode {
            bottom = format!("Search   : {}", app.search_input);
        } else if !app.search_query.is_empty() {
            bottom = format!(
                "Filter   : {}  (press '/' to edit, Enter on empty to clear)",
                app.search_query
            );
        } else if !app.status_message.is_empty() && Instant::now() < app.status_until {
            bottom = format!("Status   : {}", app.status_message);
        }
        draw_segment(
            &mut stdout,
            0,
            h - 2,
            &bottom,
            w,
            Some(COLOR_YELLOW),
            Some(COLOR_APP_BG),
            false,
            false,
        )?;

        let mut help =
            "q:quit  /:search  Enter:lock/unlock  ↑/↓ or j/k:move  PgUp/PgDn:page  Home/End  x:kill  c/m/p:sort  r:refresh".to_string();
        if max_scroll > 0 {
            let pos = min(metrics.procs.len(), app.scroll_offset + 1);
            let end = min(metrics.procs.len(), app.scroll_offset + visible.len());
            help.push_str(&format!("  [{}-{}/{}]", pos, end, metrics.procs.len()));
        }
        draw_segment(
            &mut stdout,
            0,
            h - 1,
            &help,
            w,
            Some(COLOR_TEXT),
            Some(COLOR_PANEL_BG),
            false,
            false,
        )?;

        if app.search_mode {
            let cursor_x = min(
                w.saturating_sub(1),
                ("Search   : ".chars().count() + app.search_input.chars().count()) as u16,
            );
            queue!(
                stdout,
                Show,
                SetCursorStyle::BlinkingBlock,
                MoveTo(cursor_x, h - 2)
            )?;
        } else {
            queue!(stdout, Hide)?;
        }

        stdout.flush()?;
    }

    Ok(())
}

fn main() {
    let (refresh_rate, top) = parse_args();

    let result = run_app(refresh_rate, top);

    let mut stdout = stdout();
    let _ = execute!(stdout, Show, LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();

    if let Err(err) = result {
        eprintln!("Error: {}", err);
    }
}
