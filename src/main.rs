use crossterm::cursor::{Hide, MoveTo, SetCursorStyle, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Print, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use libc::{kill, SIGKILL};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, stdout, Stdout, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{CpuExt, DiskExt, PidExt, ProcessExt, ProcessStatus, System, SystemExt};

const ASSUMED_CPU_PACKAGE_POWER_W: f32 = 65.0;
const MIN_DISPLAY_POWER_W: f32 = 0.1;
const NET_EMA_ALPHA: f64 = 0.35;
const MIN_NET_DT: f64 = 0.25;
const UI_POLL_MS: u64 = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Cpu,
    Mem,
    Power,
}

struct ProcRow {
    pid: i32,
    user: String,
    cpu_percent: f32,
    mem_percent: f32,
    power_watts: Option<f32>,
    name: String,
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

fn now_text() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

fn parse_meminfo() -> Option<(u64, u64, u64, u64)> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = 0_u64;
    let mut free = 0_u64;
    let mut buffers = 0_u64;
    let mut cached = 0_u64;

    for line in content.lines() {
        let mut parts = line.split(':');
        let key = parts.next()?.trim();
        let val_part = parts.next()?.trim();
        let kb = val_part.split_whitespace().next()?.parse::<u64>().ok()?;
        let bytes = kb.saturating_mul(1024);
        match key {
            "MemTotal" => total = bytes,
            "MemFree" => free = bytes,
            "Buffers" => buffers = bytes,
            "Cached" => cached = bytes,
            _ => {}
        }
    }

    if total == 0 {
        None
    } else {
        Some((total, free, buffers, cached))
    }
}

fn sample_memory_usage(system: &System) -> (u64, u64, f32) {
    if let Some((total, free, buffers, cached)) = parse_meminfo() {
        let used = total.saturating_sub(free.saturating_add(buffers).saturating_add(cached));
        let pct = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };
        return (used, total, pct);
    }

    let total = system.total_memory().saturating_mul(1024);
    let used = system.used_memory().saturating_mul(1024);
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
        let iface = parts.next()?.trim();
        let stats = parts.next()?.split_whitespace().collect::<Vec<_>>();
        if iface == "lo" || stats.len() < 16 {
            continue;
        }

        let recv = stats[0].parse::<u64>().ok()?;
        let sent = stats[8].parse::<u64>().ok()?;
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
            format!("Killed {} proc(s), denied {} for PID {}", killed, denied, pid),
        );
    }

    (true, format!("Killed PID {} (and {} child proc(s))", pid, killed.saturating_sub(1)))
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

fn draw_line(stdout: &mut Stdout, row: u16, text: &str, width: u16, reverse: bool, bold: bool) -> io::Result<()> {
    let w = width as usize;
    let shown = if w == 0 { "".to_string() } else { truncate_to_width(text, w) };
    let padded = if shown.chars().count() < w {
        format!("{}{}", shown, " ".repeat(w - shown.chars().count()))
    } else {
        shown
    };

    queue!(stdout, MoveTo(0, row))?;
    if reverse {
        queue!(stdout, SetAttribute(Attribute::Reverse))?;
    }
    if bold {
        queue!(stdout, SetAttribute(Attribute::Bold))?;
    }

    queue!(stdout, Print(padded))?;
    queue!(stdout, SetAttribute(Attribute::Reset))?;
    Ok(())
}

fn build_processes(system: &System, sort_key: SortKey, logical_cpus: usize, top: usize) -> (Vec<ProcRow>, usize) {
    let total_mem_kib = system.total_memory().max(1);
    let mut rows = Vec::new();

    for (pid, proc_) in system.processes() {
        if proc_.status() == ProcessStatus::Zombie {
            continue;
        }

        let cpu = proc_.cpu_usage();
        let mem_pct = (proc_.memory() as f64 / total_mem_kib as f64 * 100.0) as f32;
        let power = estimate_process_power_watts(cpu, logical_cpus);

        rows.push(ProcRow {
            pid: pid.as_u32() as i32,
            user: "-".to_string(),
            cpu_percent: cpu,
            mem_percent: mem_pct,
            power_watts: power,
            name: proc_.name().to_string(),
        });
    }

    match sort_key {
        SortKey::Cpu => rows.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal)),
        SortKey::Mem => rows.sort_by(|a, b| b.mem_percent.partial_cmp(&a.mem_percent).unwrap_or(std::cmp::Ordering::Equal)),
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

    let cpu_per_core = system.cpus().iter().map(|c| c.cpu_usage()).collect::<Vec<_>>();
    let cpu_total = if cpu_per_core.is_empty() {
        0.0
    } else {
        cpu_per_core.iter().copied().sum::<f32>() / cpu_per_core.len() as f32
    };

    let load = system.load_average();
    let uptime_text = human_uptime(system.uptime());

    let (mem_used, mem_total, mem_percent) = sample_memory_usage(system);
    let swap_total = system.total_swap().saturating_mul(1024);
    let swap_used = system.used_swap().saturating_mul(1024);
    let swap_percent = if swap_total > 0 {
        (swap_used as f64 / swap_total as f64 * 100.0) as f32
    } else {
        0.0
    };

    let (disk_total, disk_used) = if let Some(root_disk) = system
        .disks()
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .or_else(|| system.disks().iter().next())
    {
        let total = root_disk.total_space();
        let used = total.saturating_sub(root_disk.available_space());
        (total, used)
    } else {
        (0, 0)
    };
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
                            if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
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
                                app.selected_index = min(app.selected_index, metrics.procs.len() - 1);
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
                        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_add(1);
                        }
                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_sub(1);
                        }
                        KeyCode::PageDown if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_add(20);
                        }
                        KeyCode::PageUp if app.locked_pid.is_none() => {
                            app.selected_index = app.selected_index.saturating_sub(20);
                        }
                        KeyCode::Home if app.locked_pid.is_none() => app.selected_index = 0,
                        KeyCode::End if app.locked_pid.is_none() => app.selected_index = usize::MAX / 2,
                        KeyCode::Char('X') | KeyCode::Char('x') => {
                            if metrics.procs.is_empty() {
                                app.status_message = "No process selected".to_string();
                                app.status_until = Instant::now() + Duration::from_secs(2);
                            } else {
                                app.selected_index = min(app.selected_index, metrics.procs.len() - 1);
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

        if h < 8 || w < 40 {
            draw_line(
                &mut stdout,
                0,
                "Terminal too small. Resize window (min 40x8). Press q to quit.",
                w,
                false,
                false,
            )?;
            stdout.flush()?;
            continue;
        }

        let bar_w = max(10, min(40, w as usize - 38));
        let visible_rows = max(1, h as usize - 16);

        draw_line(
            &mut stdout,
            0,
            &format!(" SysWatcher • {} • {} ", metrics.host, metrics.now_text),
            w,
            true,
            false,
        )?;

        draw_line(
            &mut stdout,
            2,
            &format!(
                "CPU Total: {:5.1}%  [{}]",
                metrics.cpu_total,
                progress_bar(metrics.cpu_total, bar_w)
            ),
            w,
            false,
            false,
        )?;
        draw_line(
            &mut stdout,
            3,
            &format!(
                "Load Avg : {:.2}  {:.2}  {:.2}    Uptime: {}",
                metrics.load_1, metrics.load_5, metrics.load_15, metrics.uptime_text
            ),
            w,
            false,
            false,
        )?;

        draw_line(
            &mut stdout,
            5,
            &format!(
                "Memory   : {:5.1}%  {}/{}  [{}]",
                metrics.mem_percent,
                human_bytes(metrics.mem_used as f64, 2),
                human_bytes(metrics.mem_total as f64, 2),
                progress_bar(metrics.mem_percent, bar_w)
            ),
            w,
            false,
            false,
        )?;
        draw_line(
            &mut stdout,
            6,
            &format!(
                "Swap     : {:5.1}%  {}/{}  [{}]",
                metrics.swap_percent,
                human_bytes(metrics.swap_used as f64, 2),
                human_bytes(metrics.swap_total as f64, 2),
                progress_bar(metrics.swap_percent, bar_w)
            ),
            w,
            false,
            false,
        )?;
        draw_line(
            &mut stdout,
            7,
            &format!(
                "Disk /   : {:5.1}%  {}/{}  [{}]",
                metrics.disk_percent,
                human_bytes(metrics.disk_used as f64, 1),
                human_bytes(metrics.disk_total as f64, 1),
                progress_bar(metrics.disk_percent, bar_w)
            ),
            w,
            false,
            false,
        )?;

        draw_line(
            &mut stdout,
            9,
            &format!(
                "Network  : ↓ {}/s   ↑ {}/s   Total ↓ {} ↑ {}",
                human_bytes(metrics.net_rate_down, 1),
                human_bytes(metrics.net_rate_up, 1),
                human_bytes(metrics.net_total_down as f64, 1),
                human_bytes(metrics.net_total_up as f64, 1)
            ),
            w,
            false,
            false,
        )?;

        let cores = metrics
            .cpu_per_core
            .iter()
            .enumerate()
            .map(|(i, v)| format!("C{}:{:>4.0}%", i, v))
            .collect::<Vec<_>>()
            .join(" ");
        draw_line(&mut stdout, 10, &format!("Cores    : {}", cores), w, false, false)?;

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
        draw_line(&mut stdout, 12, &proc_header, w, false, true)?;
        draw_line(
            &mut stdout,
            13,
            "PID      USER            CPU%   MEM%     PWR   NAME",
            w,
            false,
            true,
        )?;

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

        let max_scroll = metrics.procs.len().saturating_sub(visible_rows);
        app.scroll_offset = min(app.scroll_offset, max_scroll);

        let end_ix = min(metrics.procs.len(), app.scroll_offset + visible_rows);
        let visible = &metrics.procs[app.scroll_offset..end_ix];

        let mut row = 14_u16;
        for (idx, p) in visible.iter().enumerate() {
            if row >= h.saturating_sub(2) {
                break;
            }
            let absolute_index = app.scroll_offset + idx;
            let name_w = w as usize - 46;
            let name = truncate_to_width(&p.name, max(1, name_w));
            let line = format!(
                "{:<8} {:<14} {:>5.1}  {:>5.1}  {:>6}  {}",
                p.pid,
                truncate_to_width(&p.user, 14),
                p.cpu_percent,
                p.mem_percent,
                format_power_usage(p.power_watts),
                name
            );
            draw_line(
                &mut stdout,
                row,
                &line,
                w,
                absolute_index == app.selected_index,
                false,
            )?;
            row = row.saturating_add(1);
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
        draw_line(&mut stdout, h - 2, &bottom, w, false, false)?;

        let mut help =
            "q:quit  /:search  Enter:lock/unlock  ↑/↓ or j/k:move  PgUp/PgDn:page  Home/End  x:kill  c/m/p:sort  r:refresh".to_string();
        if max_scroll > 0 {
            let pos = min(metrics.procs.len(), app.scroll_offset + 1);
            let end = min(metrics.procs.len(), app.scroll_offset + visible.len());
            help.push_str(&format!("  [{}-{}/{}]", pos, end, metrics.procs.len()));
        }
        draw_line(&mut stdout, h - 1, &help, w, true, false)?;

        if app.search_mode {
            let cursor_x = min(
                w.saturating_sub(1),
                ("Search   : ".chars().count() + app.search_input.chars().count()) as u16,
            );
            queue!(stdout, Show, SetCursorStyle::BlinkingBlock, MoveTo(cursor_x, h - 2))?;
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
