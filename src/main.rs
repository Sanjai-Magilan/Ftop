use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use libc::{kill, SIGKILL};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect as TuiRect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::cmp::{max, min};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, stdout};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{CpuExt, PidExt, ProcessExt, ProcessStatus, System, SystemExt};

mod process_controller;

const ASSUMED_CPU_PACKAGE_POWER_W: f32 = 65.0;
const MIN_DISPLAY_POWER_W: f32 = 0.1;
const NET_EMA_ALPHA: f64 = 0.35;
const MIN_NET_DT: f64 = 0.25;
const UI_POLL_MS: u64 = 50;
const METER_LABEL_WIDTH: usize = 7;
const METER_DETAIL_WIDTH: usize = 18;
const METER_STATIC_WIDTH: usize = METER_LABEL_WIDTH + METER_DETAIL_WIDTH + 55;
const COLOR_APP_BG: Color = Color::Rgb(17, 17, 27);
const COLOR_PANEL_BG: Color = Color::Rgb(30, 30, 46);
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);
const COLOR_MUTED: Color = Color::Rgb(147, 153, 178);
const COLOR_CYAN: Color = Color::Rgb(137, 220, 235);
const COLOR_PURPLE: Color = Color::Rgb(203, 166, 247);
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);
const COLOR_YELLOW: Color = Color::Rgb(249, 226, 175);
const COLOR_RED: Color = Color::Rgb(243, 139, 168);
const COLOR_SELECT_BG: Color = Color::Rgb(69, 71, 90);

mod theme {
    use super::*;

    pub fn base_style() -> Style {
        Style::default().fg(COLOR_TEXT).bg(COLOR_APP_BG)
    }

    pub fn panel_style() -> Style {
        Style::default().fg(COLOR_TEXT).bg(COLOR_PANEL_BG)
    }

    pub fn header_style() -> Style {
        panel_style().fg(COLOR_CYAN).add_modifier(Modifier::BOLD)
    }

    pub fn title_style() -> Style {
        panel_style().fg(COLOR_PURPLE).add_modifier(Modifier::BOLD)
    }

    pub fn label_style() -> Style {
        panel_style().fg(COLOR_TEXT)
    }

    pub fn muted_style() -> Style {
        panel_style().fg(COLOR_MUTED)
    }

    pub fn selected_style(style: Style) -> Style {
        style.bg(COLOR_SELECT_BG)
    }

    pub fn status_style() -> Style {
        base_style().fg(COLOR_YELLOW)
    }

    pub fn usage_style(percent: f64) -> Style {
        let fg = if percent >= 85.0 {
            COLOR_RED
        } else if percent >= 65.0 {
            COLOR_YELLOW
        } else {
            COLOR_GREEN
        };
        panel_style().fg(fg).add_modifier(Modifier::BOLD)
    }

    pub fn panel_block(title: impl Into<Line<'static>>) -> Block<'static> {
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(muted_style())
            .style(panel_style())
    }
}

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
    parent_pid: Option<i32>,
    cpu_percent: f32,
    mem_percent: f32,
    power_watts: Option<f32>,
    uptime_secs: u64,
    shared_bytes: u64,
    virtual_bytes: u64,
    resident_bytes: u64,
    priority: i64,
    name: String,
    tree_prefix: String,
    tree_marker: String,
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
    tree_view: bool,
    search_mode: bool,
    search_query: String,
    search_input: String,
    status_message: String,
    status_until: Instant,
    force_refresh: bool,
    down_rate_ema: f64,
    up_rate_ema: f64,
    pending_action: Option<PendingAction>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sort_key: SortKey::Cpu,
            scroll_offset: 0,
            selected_index: 0,
            locked_pid: None,
            tree_view: false,
            search_mode: false,
            search_query: String::new(),
            search_input: String::new(),
            status_message: String::new(),
            status_until: Instant::now(),
            force_refresh: true,
            down_rate_ema: 0.0,
            up_rate_ema: 0.0,
            pending_action: None,
        }
    }
}

#[derive(Clone)]
enum PendingAction {
    Suspend { pid: i32, msg: String },
    Resume { pid: i32, msg: String },
    Kill { pid: i32, msg: String },
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
            "PID", "CPU%", "MEM%", "PRI", "UPTIME", "SHR", "VIRT", "RES", "PWR", "PROCESS"
        ),
        ProcessTableMode::Compact => format!(
            "{:>8} {:>5}  {:>5}  {:>8} {:>9} {:>8}  {:>6}  {}",
            "PID", "CPU%", "MEM%", "SHR", "VIRT", "RES", "PWR", "PROCESS"
        ),
        ProcessTableMode::Tiny => format!(
            "{:>8} {:>5}  {:>5}  {:>8}  {}",
            "PID", "CPU%", "MEM%", "RES", "PROCESS"
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

fn compare_process_rows(a: &ProcRow, b: &ProcRow, sort_key: SortKey) -> std::cmp::Ordering {
    let ord = match sort_key {
        SortKey::Cpu => b
            .cpu_percent
            .partial_cmp(&a.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Mem => b
            .mem_percent
            .partial_cmp(&a.mem_percent)
            .unwrap_or(std::cmp::Ordering::Equal),
        SortKey::Power => b
            .power_watts
            .unwrap_or(0.0)
            .partial_cmp(&a.power_watts.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal),
    };

    ord.then_with(|| a.pid.cmp(&b.pid))
}

fn visit_tree_rows(
    pid: i32,
    prefix: String,
    marker: String,
    by_pid: &mut HashMap<i32, ProcRow>,
    children: &HashMap<i32, Vec<i32>>,
    sort_key: SortKey,
    visited: &mut HashSet<i32>,
    out: &mut Vec<ProcRow>,
) {
    if !visited.insert(pid) {
        return;
    }

    let Some(mut row) = by_pid.remove(&pid) else {
        return;
    };

    row.tree_prefix = prefix.clone();
    row.tree_marker = marker.clone();
    out.push(row);

    let next_prefix = if marker.is_empty() {
        prefix
    } else if marker == "└─ " {
        format!("{}   ", prefix)
    } else {
        format!("{}│  ", prefix)
    };

    let mut child_pids = children.get(&pid).cloned().unwrap_or_default();
    child_pids.sort_by(|left, right| {
        let Some(left_row) = by_pid.get(left) else {
            return std::cmp::Ordering::Equal;
        };
        let Some(right_row) = by_pid.get(right) else {
            return std::cmp::Ordering::Equal;
        };
        compare_process_rows(left_row, right_row, sort_key)
    });

    let last_index = child_pids.len().saturating_sub(1);
    for (index, child_pid) in child_pids.into_iter().enumerate() {
        let child_marker = if index == last_index { "└─ " } else { "├─ " };
        visit_tree_rows(
            child_pid,
            next_prefix.clone(),
            child_marker.to_string(),
            by_pid,
            children,
            sort_key,
            visited,
            out,
        );
    }
}

fn build_tree_rows(rows: Vec<ProcRow>, sort_key: SortKey) -> Vec<ProcRow> {
    let mut by_pid = rows.into_iter().map(|row| (row.pid, row)).collect::<HashMap<_, _>>();
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut roots = Vec::new();

    for (&pid, row) in &by_pid {
        match row.parent_pid {
            Some(parent_pid) if parent_pid != pid && by_pid.contains_key(&parent_pid) => {
                children.entry(parent_pid).or_default().push(pid);
            }
            _ => roots.push(pid),
        }
    }

    roots.sort_by(|left, right| {
        let Some(left_row) = by_pid.get(left) else {
            return std::cmp::Ordering::Equal;
        };
        let Some(right_row) = by_pid.get(right) else {
            return std::cmp::Ordering::Equal;
        };
        compare_process_rows(left_row, right_row, sort_key)
    });

    let mut visited = HashSet::new();
    let mut out = Vec::new();
    let last_index = roots.len().saturating_sub(1);

    for (index, pid) in roots.into_iter().enumerate() {
        let marker = if index == last_index { "" } else { "" };
        visit_tree_rows(
            pid,
            String::new(),
            marker.to_string(),
            &mut by_pid,
            &children,
            sort_key,
            &mut visited,
            &mut out,
        );
    }

    if !by_pid.is_empty() {
        let mut leftovers = by_pid.into_values().collect::<Vec<_>>();
        leftovers.sort_by(|left, right| compare_process_rows(left, right, sort_key));
        out.extend(leftovers);
    }

    out
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

fn meter_line(label: &str, percent: f32, detail: &str, width: usize) -> Line<'static> {
    let bar_w = max(10, width.saturating_sub(METER_STATIC_WIDTH));
    Line::from(vec![
        Span::styled(
            format!("{:<label_width$} ", label, label_width = METER_LABEL_WIDTH),
            theme::label_style(),
        ),
        Span::styled(
            format!("{:>5.1}%", percent),
            theme::usage_style(percent as f64),
        ),
        Span::styled(" [", theme::muted_style()),
        Span::styled(
            progress_bar(percent, bar_w),
            theme::usage_style(percent as f64),
        ),
        Span::styled("] ", theme::muted_style()),
        Span::styled(detail.to_string(), theme::muted_style()),
    ])
}

fn row_style(style: Style, selected: bool) -> Style {
    if selected {
        theme::selected_style(style)
    } else {
        style
    }
}

fn title_line(title: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(format!(" {} ", title.into()), style))
}

fn text_span(text: impl Into<String>, style: Style, selected: bool) -> Span<'static> {
    Span::styled(text.into(), row_style(style, selected))
}

fn core_line(idx: usize, cpu: f32, width: usize, selected: bool) -> Line<'static> {
    let bar_w = max(4, width.saturating_sub(11));
    Line::from(vec![
        text_span(format!("C{:02} ", idx), theme::label_style(), selected),
        text_span(
            format!("{:>3.0}%", cpu),
            theme::usage_style(cpu as f64),
            selected,
        ),
        text_span(" ", theme::muted_style(), selected),
        text_span(
            progress_bar(cpu, bar_w),
            theme::usage_style(cpu as f64),
            selected,
        ),
    ])
}

fn cpu_core_columns(panel_width: usize, core_count: usize) -> usize {
    let inner_width = panel_width.saturating_sub(2);
    let by_width = max(1, inner_width / 18);
    max(1, min(core_count.max(1), min(4, by_width)))
}

fn process_table_line(
    p: &ProcRow,
    mode: ProcessTableMode,
    width: usize,
    selected: bool,
    tree_view: bool,
) -> Line<'static> {
    let fixed_width = process_table_fixed_width(mode);
    let tree_prefix = if tree_view {
        format!("{}{}", p.tree_prefix, p.tree_marker)
    } else {
        String::new()
    };
    let name_w = width.saturating_sub(fixed_width + tree_prefix.chars().count()).max(1);
    let name = truncate_to_width(&p.name, name_w);

    match mode {
        ProcessTableMode::Full => Line::from(vec![
            text_span(format!("{:>8} ", p.pid), theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.cpu_percent),
                theme::usage_style(p.cpu_percent as f64),
                selected,
            ),
            text_span("  ", theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.mem_percent),
                theme::usage_style(p.mem_percent as f64),
                selected,
            ),
            text_span(
                format!(
                    "  {:>4}  {:>11} {:>8} {:>9} {:>8}  {:>6}  {}{}",
                    p.priority,
                    human_uptime(p.uptime_secs),
                    format_process_memory(p.shared_bytes, 8),
                    format_process_memory(p.virtual_bytes, 9),
                    format_process_memory(p.resident_bytes, 8),
                    format_power_usage(p.power_watts),
                    tree_prefix,
                    name
                ),
                theme::label_style(),
                selected,
            ),
        ]),
        ProcessTableMode::Compact => Line::from(vec![
            text_span(format!("{:>8} ", p.pid), theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.cpu_percent),
                theme::usage_style(p.cpu_percent as f64),
                selected,
            ),
            text_span("  ", theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.mem_percent),
                theme::usage_style(p.mem_percent as f64),
                selected,
            ),
            text_span(
                format!(
                    "  {:>8} {:>9} {:>8}  {:>6}  {}{}",
                    format_process_memory(p.shared_bytes, 8),
                    format_process_memory(p.virtual_bytes, 9),
                    format_process_memory(p.resident_bytes, 8),
                    format_power_usage(p.power_watts),
                    tree_prefix,
                    name
                ),
                theme::label_style(),
                selected,
            ),
        ]),
        ProcessTableMode::Tiny => Line::from(vec![
            text_span(format!("{:>8} ", p.pid), theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.cpu_percent),
                theme::usage_style(p.cpu_percent as f64),
                selected,
            ),
            text_span("  ", theme::muted_style(), selected),
            text_span(
                format!("{:>5.1}", p.mem_percent),
                theme::usage_style(p.mem_percent as f64),
                selected,
            ),
            text_span(
                format!(
                    "  {:>8}  {}{}",
                    format_process_memory(p.resident_bytes, 8),
                    tree_prefix,
                    name
                ),
                theme::label_style(),
                selected,
            ),
        ]),
    }
}

fn render_cpu_panel(frame: &mut Frame, area: TuiRect, metrics: &RuntimeMetrics) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let core_cols = cpu_core_columns(area.width as usize, metrics.cpu_per_core.len());
    let core_col_w = max(13, inner_width / core_cols);
    let mut lines = vec![
        meter_line(
            "Total",
            metrics.cpu_total,
            &format!(
                "load {:.2} {:.2} {:.2}",
                metrics.load_1, metrics.load_5, metrics.load_15
            ),
            inner_width,
        ),
        Line::from(vec![
            Span::styled("Uptime  ", theme::label_style()),
            Span::styled(metrics.uptime_text.clone(), theme::muted_style()),
        ]),
    ];

    for (row_idx, row_cpus) in metrics.cpu_per_core.chunks(core_cols).enumerate() {
        let mut spans = Vec::new();
        for (col, cpu) in row_cpus.iter().enumerate() {
            if col > 0 {
                spans.push(Span::styled("  ", theme::muted_style()));
            }
            let idx = row_idx * core_cols + col;
            let line = core_line(idx, *cpu, core_col_w, false);
            for span in line.spans {
                spans.push(span);
            }
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::panel_block(title_line("CPU", theme::header_style())))
            .style(theme::panel_style()),
        area,
    );
}

fn render_system_panel(frame: &mut Frame, area: TuiRect, metrics: &RuntimeMetrics) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let lines = vec![
        meter_line(
            "Memory",
            metrics.mem_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.mem_used as f64, 2),
                human_bytes(metrics.mem_total as f64, 2)
            ),
            inner_width,
        ),
        meter_line(
            "Swap",
            metrics.swap_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.swap_used as f64, 1),
                human_bytes(metrics.swap_total as f64, 1)
            ),
            inner_width,
        ),
        meter_line(
            "Disk /",
            metrics.disk_percent,
            &format!(
                "{}/{}",
                human_bytes(metrics.disk_used as f64, 1),
                human_bytes(metrics.disk_total as f64, 1)
            ),
            inner_width,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("Down ", theme::label_style()),
            Span::styled(
                format!("{:>10}/s", human_bytes(metrics.net_rate_down, 1)),
                theme::usage_style(0.0),
            ),
            Span::styled("    Up ", theme::label_style()),
            Span::styled(
                format!("{:>10}/s", human_bytes(metrics.net_rate_up, 1)),
                theme::usage_style(0.0),
            ),
        ]),
        Line::from(vec![
            Span::styled("Total ↓ ", theme::label_style()),
            Span::styled(
                format!("{:>10}", human_bytes(metrics.net_total_down as f64, 1)),
                theme::muted_style(),
            ),
            Span::styled("     ↑ ", theme::label_style()),
            Span::styled(
                format!("{:>10}", human_bytes(metrics.net_total_up as f64, 1)),
                theme::muted_style(),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::panel_block(title_line(
                "Memory • Disk • Net",
                theme::title_style(),
            )))
            .style(theme::panel_style()),
        area,
    );
}

fn render_process_panel(
    frame: &mut Frame,
    area: TuiRect,
    app: &mut AppState,
    metrics: &RuntimeMetrics,
) {
    let sort_label = match app.sort_key {
        SortKey::Cpu => "CPU",
        SortKey::Mem => "MEM",
        SortKey::Power => "PWR*",
    };
    let view_label = if app.tree_view { "TREE" } else { "FLAT" };

    let mut proc_header = format!(
        "Processes: {} total | Showing {} by {} | {}",
        metrics.proc_total,
        metrics.procs.len(),
        sort_label,
        view_label
    );
    if !app.search_query.is_empty() {
        proc_header.push_str(&format!(" | /{}", app.search_query));
    }
    if let Some(pid) = app.locked_pid {
        proc_header.push_str(&format!(" | LOCK PID {}", pid));
    }

    // Add selected process state indicator
    if !metrics.procs.is_empty() && app.selected_index < metrics.procs.len() {
        let selected_pid = metrics.procs[app.selected_index].pid;
        match process_controller::ProcessController::get_process_status(selected_pid) {
            Ok(is_stopped) => {
                let state = if is_stopped { "STOPPED" } else { "RUNNING" };
                proc_header.push_str(&format!(" | SELECTED: {} ({})", selected_pid, state));
            }
            Err(_) => {
                proc_header.push_str(&format!(" | SELECTED: {} (?)", selected_pid));
            }
        }
    }

    let table_width = area.width.saturating_sub(2) as usize;
    let table_mode = process_table_mode(table_width);
    let visible_rows = max(1, area.height.saturating_sub(3) as usize);

    if metrics.procs.is_empty() {
        app.selected_index = 0;
    } else {
        app.selected_index = min(app.selected_index, metrics.procs.len() - 1);
    }

    if app.selected_index < app.scroll_offset {
        app.scroll_offset = app.selected_index;
    } else if app.selected_index >= app.scroll_offset + visible_rows {
        app.scroll_offset = app.selected_index.saturating_sub(visible_rows - 1);
    }
    app.scroll_offset = min(
        app.scroll_offset,
        metrics.procs.len().saturating_sub(visible_rows),
    );

    let end_ix = min(metrics.procs.len(), app.scroll_offset + visible_rows);
    let visible = &metrics.procs[app.scroll_offset..end_ix];
    let mut lines = vec![Line::from(Span::styled(
        process_table_header(table_mode),
        theme::title_style(),
    ))];

    for (idx, p) in visible.iter().enumerate() {
        let absolute_index = app.scroll_offset + idx;
        lines.push(process_table_line(
            p,
            table_mode,
            table_width,
            absolute_index == app.selected_index,
            app.tree_view,
        ));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(theme::panel_block(title_line(
                proc_header,
                theme::header_style(),
            )))
            .style(theme::panel_style()),
        area,
    );
}

fn render_dashboard(frame: &mut Frame, app: &mut AppState, metrics: &RuntimeMetrics) {
    let area = frame.size();
    frame.render_widget(Block::default().style(theme::base_style()), area);

    if area.height < 18 || area.width < 72 {
        frame.render_widget(
            Paragraph::new("Terminal too small. Resize window (min 72x18). Press q to quit.")
                .style(theme::base_style().fg(COLOR_RED)),
            area,
        );
        return;
    }

    let estimated_cpu_panel_width = area.width.saturating_sub(1) as usize * 60 / 100;
    let estimated_cpu_cols = cpu_core_columns(estimated_cpu_panel_width, metrics.cpu_per_core.len());
    let estimated_cpu_rows = (metrics.cpu_per_core.len() + estimated_cpu_cols - 1) / estimated_cpu_cols;
    let top_h = max(8, estimated_cpu_rows + 4) as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(top_h),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Length(1),
            Constraint::Percentage(40),
        ])
        .split(chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" SysWatcher ", theme::title_style().bg(COLOR_APP_BG)),
            Span::styled("• ", theme::muted_style().bg(COLOR_APP_BG)),
            Span::styled(metrics.host.clone(), theme::header_style().bg(COLOR_APP_BG)),
            Span::styled(" • ", theme::muted_style().bg(COLOR_APP_BG)),
            Span::styled(
                metrics.now_text.clone(),
                theme::muted_style().bg(COLOR_APP_BG),
            ),
        ]))
        .style(theme::base_style()),
        chunks[0],
    );

    render_cpu_panel(frame, top_chunks[0], metrics);
    render_system_panel(frame, top_chunks[2], metrics);
    render_process_panel(frame, chunks[3], app, metrics);

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
    frame.render_widget(
        Paragraph::new(bottom).style(theme::status_style()),
        chunks[4],
    );

    let visible_rows = max(1, chunks[3].height.saturating_sub(3) as usize);
    let max_scroll = metrics.procs.len().saturating_sub(visible_rows);
    let mut help =
        "q:quit  /:search  t:tree  s:sleep  w:wake  Enter:lock/unlock  ↑/↓ or j/k:move  PgUp/PgDn:page  Home/End  x:kill  c/m/p:sort  r:refresh"
            .to_string();
    if max_scroll > 0 {
        let pos = min(metrics.procs.len(), app.scroll_offset + 1);
        let end = min(metrics.procs.len(), app.scroll_offset + visible_rows);
        help.push_str(&format!("  [{}-{}/{}]", pos, end, metrics.procs.len()));
    }
    frame.render_widget(
        Paragraph::new(help)
            .style(theme::panel_style())
            .wrap(Wrap { trim: true }),
        chunks[5],
    );

    // Render confirmation modal if pending_action is set
    if let Some(pa) = &app.pending_action {
        let area = frame.size();
        let popup_w = (area.width as u16).saturating_mul(50) / 100;
        let popup_h = 7u16;
        let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let popup_rect = TuiRect::new(popup_x, popup_y, popup_w, popup_h);

        frame.render_widget(Clear, popup_rect);
        frame.render_widget(
            Block::default()
                .style(theme::panel_style())
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_RED)),
            popup_rect,
        );

        let mut lines = Vec::new();
        match pa {
            PendingAction::Suspend { pid, msg } => {
                lines.push(Line::from(Span::styled(
                    format!("Confirm suspend PID {}", pid),
                    theme::header_style(),
                )));
                lines.push(Line::from(msg.clone()));
            }
            PendingAction::Resume { pid, msg } => {
                lines.push(Line::from(Span::styled(
                    format!("Confirm resume PID {}", pid),
                    theme::header_style(),
                )));
                lines.push(Line::from(msg.clone()));
            }
            PendingAction::Kill { pid, msg } => {
                lines.push(Line::from(Span::styled(
                    format!("Confirm kill PID {}", pid),
                    theme::header_style(),
                )));
                lines.push(Line::from(msg.clone()));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Press Enter to confirm, Esc to cancel"));

        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title("Confirm")
                        .border_style(Style::default().fg(COLOR_RED))
                        .style(theme::panel_style()),
                )
                .style(theme::panel_style()),
            popup_rect,
        );
    }

    if app.search_mode {
        let cursor_x = min(
            chunks[4].right().saturating_sub(1),
            chunks[4].x + ("Search   : ".chars().count() + app.search_input.chars().count()) as u16,
        );
        frame.set_cursor(cursor_x, chunks[4].y);
    }
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
        let parent_pid = proc_.parent().map(|parent| parent.as_u32() as i32);
        let resident_bytes = proc_.memory();
        let virtual_bytes = proc_.virtual_memory();
        let shared_bytes = read_shared_bytes(pid_i32, page_size);
        let priority = read_process_priority(pid_i32);

        rows.push(ProcRow {
            pid: pid_i32,
            parent_pid,
            cpu_percent: cpu,
            mem_percent: mem_pct,
            power_watts: power,
            uptime_secs: proc_.run_time(),
            shared_bytes,
            virtual_bytes,
            resident_bytes,
            priority,
            name: proc_.name().to_string(),
            tree_prefix: String::new(),
            tree_marker: String::new(),
        });
    }

    rows.sort_by(|a, b| compare_process_rows(a, b, sort_key));

    let total = rows.len();
    (rows, total)
}

fn sample_metrics(
    system: &mut System,
    sort_key: SortKey,
    search_query: &str,
    tree_view: bool,
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

    let (sampled, proc_total) = build_processes(system, sort_key, logical_cpus);
    let ordered = if tree_view {
        build_tree_rows(sampled, sort_key)
    } else {
        sampled
    };
    let mut filtered = filter_processes(ordered, search_query);
    if top > 0 {
        filtered.truncate(top);
    }

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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

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
        app.tree_view,
        top,
        logical_cpus,
        &mut prev_net,
        &mut prev_time,
        &mut app.down_rate_ema,
        &mut app.up_rate_ema,
    );

    let mut last_sample = Instant::now();
    let mut needs_draw = true;

    loop {
        let status_was_visible =
            !app.status_message.is_empty() && Instant::now() < app.status_until;

        if event::poll(Duration::from_millis(UI_POLL_MS))? {
            match event::read()? {
                Event::Key(key) => {
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
                        // If a confirmation modal is active, handle its keys first
                        if let Some(pa) = app.pending_action.clone() {
                            match key.code {
                                KeyCode::Enter => {
                                    match pa {
                                        PendingAction::Suspend { pid, .. } => {
                                            match process_controller::ProcessController::suspend_process(pid) {
                                                Ok(()) => {
                                                    app.status_message = format!("Suspended PID {}", pid);
                                                    app.status_until = Instant::now() + Duration::from_secs(2);
                                                    app.force_refresh = true;
                                                }
                                                Err(e) => {
                                                    app.status_message = format!("Failed to suspend: {}", e);
                                                    app.status_until = Instant::now() + Duration::from_millis(2800);
                                                }
                                            }
                                        }
                                        PendingAction::Resume { pid, .. } => {
                                            match process_controller::ProcessController::resume_process(pid) {
                                                Ok(()) => {
                                                    app.status_message = format!("Resumed PID {}", pid);
                                                    app.status_until = Instant::now() + Duration::from_secs(2);
                                                    app.force_refresh = true;
                                                }
                                                Err(e) => {
                                                    app.status_message = format!("Failed to resume: {}", e);
                                                    app.status_until = Instant::now() + Duration::from_millis(2800);
                                                }
                                            }
                                        }
                                        PendingAction::Kill { pid, .. } => {
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
                                    app.pending_action = None;
                                }
                                KeyCode::Esc => {
                                    app.pending_action = None;
                                    app.status_message = "Action cancelled".to_string();
                                    app.status_until = Instant::now() + Duration::from_secs(1);
                                }
                                _ => {}
                            }
                            needs_draw = true;
                            continue;
                        }

                        match key.code {
                            KeyCode::Char('q') | KeyCode::Char('Q') => break,
                            KeyCode::Char('/') => {
                                app.search_mode = true;
                                app.search_input = app.search_query.clone();
                            }
                            KeyCode::Char('t') | KeyCode::Char('T') => {
                                app.tree_view = !app.tree_view;
                                app.selected_index = 0;
                                app.scroll_offset = 0;
                                app.force_refresh = true;
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
                                    let warning = format!(
                                        "This will terminate PID {} and its children using SIGKILL. Press Enter to confirm, Esc to cancel",
                                        pid
                                    );
                                    app.pending_action = Some(PendingAction::Kill {
                                        pid,
                                        msg: warning.clone(),
                                    });
                                    app.status_message = format!("Confirm: {}", warning);
                                    app.status_until = Instant::now() + Duration::from_secs(8);
                                }
                            }
                            KeyCode::Char('S') | KeyCode::Char('s') => {
                                if metrics.procs.is_empty() {
                                    app.status_message = "No process selected".to_string();
                                    app.status_until = Instant::now() + Duration::from_secs(2);
                                } else {
                                    app.selected_index =
                                        min(app.selected_index, metrics.procs.len() - 1);
                                    let pid = metrics.procs[app.selected_index].pid;
                                    if let Some(msg) = process_controller::suspend_conflict(pid) {
                                        app.pending_action = Some(PendingAction::Suspend { pid, msg: msg.clone() });
                                        app.status_message = format!("Confirm: {}. Press Enter to proceed, Esc to cancel", msg);
                                        app.status_until = Instant::now() + Duration::from_secs(8);
                                    } else {
                                        match process_controller::ProcessController::suspend_process(pid) {
                                            Ok(()) => {
                                                app.status_message = format!("Suspended PID {}", pid);
                                                app.status_until = Instant::now() + Duration::from_secs(2);
                                                app.force_refresh = true;
                                            }
                                            Err(e) => {
                                                app.status_message = format!("Failed to suspend: {}", e);
                                                app.status_until = Instant::now() + Duration::from_millis(2800);
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('W') | KeyCode::Char('w') => {
                                if metrics.procs.is_empty() {
                                    app.status_message = "No process selected".to_string();
                                    app.status_until = Instant::now() + Duration::from_secs(2);
                                } else {
                                    app.selected_index =
                                        min(app.selected_index, metrics.procs.len() - 1);
                                    let pid = metrics.procs[app.selected_index].pid;
                                    if let Some(msg) = process_controller::resume_conflict(pid) {
                                        app.pending_action = Some(PendingAction::Resume { pid, msg: msg.clone() });
                                        app.status_message = format!("Confirm: {}. Press Enter to proceed, Esc to cancel", msg);
                                        app.status_until = Instant::now() + Duration::from_secs(8);
                                    } else {
                                        match process_controller::ProcessController::resume_process(pid) {
                                            Ok(()) => {
                                                app.status_message = format!("Resumed PID {}", pid);
                                                app.status_until = Instant::now() + Duration::from_secs(2);
                                                app.force_refresh = true;
                                            }
                                            Err(e) => {
                                                app.status_message = format!("Failed to resume: {}", e);
                                                app.status_until = Instant::now() + Duration::from_millis(2800);
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    needs_draw = true;
                }
                Event::Resize(_, _) => needs_draw = true,
                _ => {}
            }
        }

        let metrics_interval = Duration::from_secs_f64(refresh_rate.max(0.2));
        if app.force_refresh || last_sample.elapsed() >= metrics_interval {
            metrics = sample_metrics(
                &mut system,
                app.sort_key,
                &app.search_query,
                app.tree_view,
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
            needs_draw = true;
        }

        let status_is_visible = !app.status_message.is_empty() && Instant::now() < app.status_until;
        if status_was_visible && !status_is_visible {
            needs_draw = true;
        }

        if needs_draw {
            terminal.draw(|frame| render_dashboard(frame, &mut app, &metrics))?;
            if app.search_mode {
                terminal.show_cursor()?;
            } else {
                terminal.hide_cursor()?;
            }
            needs_draw = false;
        }
    }

    Ok(())
}

/// Example demonstrating ProcessController usage.
///
/// This can be integrated into the interactive UI loop. For example:
/// - Press 's' to suspend the selected process
/// - Press 'c' (or 'r') to resume the selected process
/// - Check process status in the process table display
///
/// # Example Usage
/// ```
/// // In the main event loop, after capturing the selected process:
/// use process_controller::ProcessController;
///
/// let selected_pid = metrics.procs[app.selected_index].pid;
///
/// // Suspend
/// match ProcessController::suspend_process(selected_pid) {
///     Ok(()) => app.status_message = format!("Suspended PID {}", selected_pid),
///     Err(e) => app.status_message = format!("Failed: {}", e),
/// }
///
/// // Check status
/// match ProcessController::get_process_status(selected_pid) {
///     Ok(is_stopped) => {
///         if is_stopped {
///             println!("Process is stopped");
///         }
///     }
///     Err(e) => eprintln!("Error: {}", e),
/// }
///
/// // Resume
/// match ProcessController::resume_process(selected_pid) {
///     Ok(()) => app.status_message = format!("Resumed PID {}", selected_pid),
///     Err(e) => app.status_message = format!("Failed: {}", e),
/// }
/// ```
#[allow(dead_code)]
fn example_process_controller_usage() {
    use process_controller::ProcessController;

    // Example: suspend and then resume a process (would need a real PID)
    let example_pid = std::process::id() as i32;

    // Check the current status
    match ProcessController::get_process_status(example_pid) {
        Ok(is_stopped) => {
            if is_stopped {
                println!("Process {} is in stopped state", example_pid);
                // Resume it
                if let Err(e) = ProcessController::resume_process(example_pid) {
                    eprintln!("Failed to resume: {}", e);
                }
            } else {
                println!("Process {} is running", example_pid);
            }
        }
        Err(e) => eprintln!("Failed to check status: {}", e),
    }
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
