import argparse
import curses
import os
import socket
import time
from datetime import datetime
from typing import Dict, List, Optional, Tuple

import psutil


ASSUMED_CPU_PACKAGE_POWER_W = 65.0
MIN_DISPLAY_POWER_W = 0.1
NET_EMA_ALPHA = 0.35
MIN_NET_DT = 0.25
UI_POLL_MS = 50


def human_bytes(value: float, decimals: int = 1) -> str:
    units = ["B", "KB", "MB", "GB", "TB", "PB"]
    i = 0
    while value >= 1024 and i < len(units) - 1:
        value /= 1024
        i += 1
    return f"{value:.{decimals}f}{units[i]}"


def human_uptime(seconds: float) -> str:
    seconds = int(seconds)
    days, seconds = divmod(seconds, 86400)
    hours, seconds = divmod(seconds, 3600)
    minutes, seconds = divmod(seconds, 60)
    if days > 0:
        return f"{days}d {hours:02d}:{minutes:02d}:{seconds:02d}"
    return f"{hours:02d}:{minutes:02d}:{seconds:02d}"


def progress_bar(percent: float, width: int) -> str:
    width = max(10, width)
    fill = int((percent / 100.0) * width)
    fill = max(0, min(fill, width))
    return "█" * fill + "░" * (width - fill)


def estimate_process_power_watts(cpu_percent: float, logical_cpus: int) -> Optional[float]:
    """Estimate process power in watts from CPU usage.

    This is an approximation based on an assumed total CPU package power.
    """
    if logical_cpus <= 0:
        return None

    cpu_percent = cpu_percent or 0.0
    if cpu_percent <= 0.0:
        return None

    usage_fraction = cpu_percent / (100.0 * logical_cpus)
    watts = max(0.0, usage_fraction * ASSUMED_CPU_PACKAGE_POWER_W)
    if watts < MIN_DISPLAY_POWER_W:
        return None
    return watts


def format_power_usage(watts: Optional[float]) -> str:
    if watts is None:
        return "-"
    return f"{watts:.1f}W"


def kill_process_tree(pid: int) -> Tuple[bool, str]:
    """Kill a process and its children. Returns (success, message)."""
    if pid <= 0:
        return False, "Invalid process id"

    try:
        parent = psutil.Process(pid)
    except psutil.NoSuchProcess:
        return True, f"PID {pid} no longer exists"
    except psutil.AccessDenied:
        return False, f"Permission denied for PID {pid}"
    except psutil.Error:
        return False, f"Failed to access PID {pid}"

    killed = 0
    denied = 0
    targets: List[psutil.Process] = []

    try:
        children = parent.children(recursive=True)
    except (psutil.NoSuchProcess, psutil.ZombieProcess):
        children = []
    except psutil.AccessDenied:
        children = []

    # Kill deepest children first.
    for proc in reversed(children):
        try:
            proc.kill()
            targets.append(proc)
            killed += 1
        except (psutil.NoSuchProcess, psutil.ZombieProcess):
            continue
        except psutil.AccessDenied:
            denied += 1

    try:
        parent.kill()
        targets.append(parent)
        killed += 1
    except (psutil.NoSuchProcess, psutil.ZombieProcess):
        pass
    except psutil.AccessDenied:
        denied += 1

    if targets:
        try:
            _, alive = psutil.wait_procs(targets, timeout=0.8)
        except psutil.Error:
            alive = []
        if alive:
            return False, f"Kill incomplete for PID {pid} ({len(alive)} still alive)"

    if killed == 0:
        if denied > 0:
            return False, f"Permission denied for PID {pid}"
        return True, f"PID {pid} no longer exists"

    if denied > 0:
        return False, f"Killed {killed} proc(s), denied {denied} for PID {pid}"
    return True, f"Killed PID {pid} (and {max(0, killed - 1)} child proc(s))"


def prime_measurements() -> None:
    """Prime non-blocking CPU counters to avoid first-sample zeros."""
    psutil.cpu_percent(interval=None, percpu=True)
    for proc in psutil.process_iter(["pid"]):
        try:
            proc.cpu_percent(interval=None)
        except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.ZombieProcess):
            continue


def sample_cpu_usage() -> Tuple[float, List[float]]:
    """Return total CPU% and per-core CPU% using one consistent sample."""
    per_core = psutil.cpu_percent(interval=None, percpu=True)
    if not per_core:
        return 0.0, []
    total = sum(per_core) / len(per_core)
    return total, per_core


def sample_memory_usage():
    """Return Linux-like actual memory usage.

    Preferred formula (as requested):
        used = total - free - buffers - cached

    On Linux we read /proc/meminfo directly for correctness. If fields are
    missing or not on Linux, we safely fall back to psutil values.
    """
    vm = psutil.virtual_memory()

    total = float(vm.total or 0)
    free = float(getattr(vm, "free", 0) or 0)
    buffers = float(getattr(vm, "buffers", 0) or 0)
    cached = float(getattr(vm, "cached", 0) or 0)

    # Linux-first: source values directly from /proc/meminfo to align closely
    # with native system semantics.
    if os.path.exists("/proc/meminfo"):
        meminfo: Dict[str, float] = {}
        try:
            with open("/proc/meminfo", "r", encoding="utf-8") as f:
                for line in f:
                    if ":" not in line:
                        continue
                    key, value_part = line.split(":", 1)
                    value_tokens = value_part.strip().split()
                    if not value_tokens:
                        continue
                    # Values are in kB in /proc/meminfo.
                    meminfo[key] = float(value_tokens[0]) * 1024.0
        except OSError:
            meminfo = {}

        total = float(meminfo.get("MemTotal", total) or total)
        free = float(meminfo.get("MemFree", free) or free)
        buffers = float(meminfo.get("Buffers", buffers) or buffers)
        cached = float(meminfo.get("Cached", cached) or cached)

    used_actual = max(0.0, total - free - buffers - cached)

    # Fallback for environments with incomplete fields.
    if total > 0 and used_actual <= 0.0:
        used_actual = max(0.0, total - float(getattr(vm, "available", 0) or 0))

    percent_actual = (used_actual / total * 100.0) if total > 0 else 0.0
    return used_actual, total, percent_actual, vm


def sample_network_rates(net_prev, net_prev_time: float, ema_down: float, ema_up: float):
    """Return current net counters and smoothed upload/download rates."""
    net_now = psutil.net_io_counters()
    now_t = time.time()
    dt = now_t - net_prev_time
    if dt < MIN_NET_DT:
        dt = MIN_NET_DT

    down_rate = max(0.0, (net_now.bytes_recv - net_prev.bytes_recv) / dt)
    up_rate = max(0.0, (net_now.bytes_sent - net_prev.bytes_sent) / dt)

    if ema_down <= 0.0:
        ema_down = down_rate
    else:
        ema_down = NET_EMA_ALPHA * down_rate + (1.0 - NET_EMA_ALPHA) * ema_down

    if ema_up <= 0.0:
        ema_up = up_rate
    else:
        ema_up = NET_EMA_ALPHA * up_rate + (1.0 - NET_EMA_ALPHA) * ema_up

    return net_now, now_t, ema_down, ema_up


def get_top_processes(limit: int, sort_key: str, logical_cpus: int):
    """Sample and return process rows plus total visible process count."""
    procs = []
    for proc in psutil.process_iter(["pid", "name", "username", "memory_percent", "status"]):
        try:
            info = proc.info
            if info.get("status") == psutil.STATUS_ZOMBIE:
                continue

            cpu_percent = proc.cpu_percent(interval=None)
            mem_percent = info.get("memory_percent") or 0.0

            info["cpu_percent"] = cpu_percent
            info["memory_percent"] = mem_percent
            info["power_watts"] = estimate_process_power_watts(cpu_percent, logical_cpus)
            procs.append(info)
        except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.ZombieProcess):
            continue

    if sort_key == "mem":
        procs.sort(key=lambda p: p.get("memory_percent") or 0.0, reverse=True)
    elif sort_key == "power":
        procs.sort(key=lambda p: p.get("power_watts") or 0.0, reverse=True)
    else:
        procs.sort(key=lambda p: p.get("cpu_percent") or 0.0, reverse=True)

    total_count = len(procs)
    if limit <= 0:
        return procs, total_count
    return procs[:limit], total_count


def filter_processes(procs: List[dict], query: str) -> List[dict]:
    """Filter processes by name or PID substring."""
    q = (query or "").strip().lower()
    if not q:
        return procs

    filtered = []
    for p in procs:
        pid_text = str(p.get("pid", ""))
        name_text = (p.get("name") or "").lower()
        if q in pid_text or q in name_text:
            filtered.append(p)
    return filtered


def safe_addnstr(stdscr, y: int, x: int, text: str, width: int, attr: int = 0) -> None:
    """Safely draw text without throwing on small terminal sizes."""
    height, screen_w = stdscr.getmaxyx()
    if y < 0 or y >= height or x < 0 or x >= screen_w:
        return

    available = min(width, screen_w - x)
    if available <= 0:
        return

    try:
        stdscr.addnstr(y, x, text, available, attr)
    except curses.error:
        # Ignore drawing errors on edge cases (tiny terminal / resize race).
        pass


def draw(stdscr, refresh_rate: float, proc_count: int):
    try:
        curses.curs_set(0)
    except curses.error:
        pass

    stdscr.nodelay(True)
    stdscr.keypad(True)
    # Fast UI polling; metric sampling runs on its own cadence.
    stdscr.timeout(UI_POLL_MS)
    try:
        # Make standalone ESC react quickly (instead of waiting for
        # function-key escape sequence timeout).
        curses.set_escdelay(25)
    except (AttributeError, curses.error):
        pass

    # Prime CPU counters so initial readings are meaningful.
    prime_measurements()

    sort_key = "cpu"
    scroll_offset = 0
    selected_index = 0
    locked_pid = None
    search_mode = False
    search_query = ""
    search_input = ""
    status_message = ""
    status_until = 0.0
    net_prev = psutil.net_io_counters()
    net_prev_time = time.time()
    logical_cpus = psutil.cpu_count(logical=True) or 1
    host = socket.gethostname()

    last_sample_time = 0.0
    force_refresh = True
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    cpu_total = 0.0
    cpu_per_core = [0.0] * logical_cpus
    load_avg = (0.0, 0.0, 0.0)
    vm = psutil.virtual_memory()
    mem_used_actual = 0.0
    mem_total_actual = float(vm.total or 0)
    mem_percent_actual = 0.0
    sm = psutil.swap_memory()
    disk = psutil.disk_usage("/")
    uptime = "00:00:00"
    down_rate_ema = 0.0
    up_rate_ema = 0.0
    net_now = net_prev
    proc_total = 0
    procs = []
    sampled_procs = []

    while True:
        height, width = stdscr.getmaxyx()
        visible_rows = max(1, height - 16)
        key = stdscr.getch()

        if key in (ord("q"), ord("Q")):
            break
        if search_mode:
            if key in (27,):  # ESC
                search_mode = False
                search_input = ""
            elif key in (curses.KEY_ENTER, 10, 13):
                search_query = search_input.strip()
                search_mode = False
                search_input = ""
                selected_index = 0
                scroll_offset = 0
            elif key in (curses.KEY_BACKSPACE, 127, 8):
                search_input = search_input[:-1]
            elif key == 21:  # Ctrl+U
                search_input = ""
            elif 32 <= key <= 126:
                search_input += chr(key)
        elif key == ord("/"):
            search_mode = True
            search_input = search_query
        elif key in (ord("c"), ord("C")):
            sort_key = "cpu"
            scroll_offset = 0
            force_refresh = True
        elif key in (ord("m"), ord("M")):
            sort_key = "mem"
            scroll_offset = 0
            force_refresh = True
        elif key in (ord("p"), ord("P")):
            sort_key = "power"
            scroll_offset = 0
            force_refresh = True
        elif key in (ord("r"), ord("R")):
            force_refresh = True
        elif key in (curses.KEY_ENTER, 10, 13):
            if not procs:
                status_message = "No process to lock"
                status_until = time.time() + 2.0
            else:
                selected_index = max(0, min(selected_index, len(procs) - 1))
                sel_pid = procs[selected_index].get("pid")
                if locked_pid == sel_pid:
                    locked_pid = None
                    status_message = "Selection unlocked"
                    status_until = time.time() + 2.0
                else:
                    locked_pid = sel_pid
                    status_message = f"Locked on PID {locked_pid}"
                    status_until = time.time() + 2.0
        elif locked_pid is None and key in (curses.KEY_DOWN, ord("j"), ord("J")):
            selected_index += 1
        elif locked_pid is None and key in (curses.KEY_UP, ord("k")):
            selected_index -= 1
        elif locked_pid is None and key == curses.KEY_NPAGE:  # Page Down
            selected_index += visible_rows
        elif locked_pid is None and key == curses.KEY_PPAGE:  # Page Up
            selected_index -= visible_rows
        elif locked_pid is None and key == curses.KEY_HOME:
            selected_index = 0
        elif locked_pid is None and key == curses.KEY_END:
            selected_index = 10**9
        elif key in (ord("K"),):
            if not procs:
                status_message = "No process selected"
                status_until = time.time() + 2.0
            else:
                selected_index = max(0, min(selected_index, len(procs) - 1))
                pid = procs[selected_index].get("pid", 0)
                try:
                    pid_int = int(pid)
                except (TypeError, ValueError):
                    pid_int = 0

                if pid_int <= 0:
                    status_message = "Invalid process id"
                    status_until = time.time() + 2.0
                else:
                    ok, msg = kill_process_tree(pid_int)
                    status_message = msg
                    status_until = time.time() + (2.0 if ok else 2.8)
                    force_refresh = True

        now_t = time.time()
        metrics_interval = max(0.2, refresh_rate)
        if force_refresh or (now_t - last_sample_time) >= metrics_interval:
            now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
            cpu_total, cpu_per_core = sample_cpu_usage()
            load_avg = os.getloadavg() if hasattr(os, "getloadavg") else (0.0, 0.0, 0.0)

            mem_used_actual, mem_total_actual, mem_percent_actual, vm = sample_memory_usage()
            sm = psutil.swap_memory()
            disk = psutil.disk_usage("/")
            boot_time = psutil.boot_time()
            uptime = human_uptime(time.time() - boot_time)

            net_now, net_prev_time, down_rate_ema, up_rate_ema = sample_network_rates(
                net_prev, net_prev_time, down_rate_ema, up_rate_ema
            )
            net_prev = net_now

            current_limit = 0 if proc_count == 0 else proc_count
            sampled_procs, proc_total = get_top_processes(current_limit, sort_key, logical_cpus)
            procs = filter_processes(sampled_procs, search_query)

            if locked_pid is not None:
                exists_any = any(p.get("pid") == locked_pid for p in sampled_procs)
                if not exists_any:
                    status_message = f"Locked PID {locked_pid} exited"
                    status_until = time.time() + 2.0
                    locked_pid = None
                else:
                    locked_index = next((i for i, p in enumerate(procs) if p.get("pid") == locked_pid), -1)
                    if locked_index >= 0:
                        selected_index = locked_index

            last_sample_time = now_t
            force_refresh = False

        stdscr.erase()

        if height < 8 or width < 40:
            safe_addnstr(stdscr, 0, 0, "Terminal too small. Resize window (min 40x8). Press q to quit.", width)
            stdscr.noutrefresh()
            curses.doupdate()
            continue

        bar_w = max(10, min(40, width - 38))

        title = f" SysWatcher • {host} • {now} "
        safe_addnstr(stdscr, 0, 0, title.ljust(width), width, curses.A_REVERSE)

        safe_addnstr(
            stdscr,
            2,
            0,
            f"CPU Total: {cpu_total:5.1f}%  [{progress_bar(cpu_total, bar_w)}]",
            width,
        )
        safe_addnstr(
            stdscr,
            3,
            0,
            f"Load Avg : {load_avg[0]:.2f}  {load_avg[1]:.2f}  {load_avg[2]:.2f}    Uptime: {uptime}",
            width,
        )

        safe_addnstr(
            stdscr,
            5,
            0,
            f"Memory   : {mem_percent_actual:5.1f}%  {human_bytes(mem_used_actual, 2)}/{human_bytes(mem_total_actual, 2)}  [{progress_bar(mem_percent_actual, bar_w)}]",
            width,
        )
        safe_addnstr(
            stdscr,
            6,
            0,
            f"Swap     : {sm.percent:5.1f}%  {human_bytes(sm.used, 2)}/{human_bytes(sm.total, 2)}  [{progress_bar(sm.percent, bar_w)}]",
            width,
        )
        safe_addnstr(
            stdscr,
            7,
            0,
            f"Disk /   : {disk.percent:5.1f}%  {human_bytes(disk.used)}/{human_bytes(disk.total)}  [{progress_bar(disk.percent, bar_w)}]",
            width,
        )

        safe_addnstr(
            stdscr,
            9,
            0,
            f"Network  : ↓ {human_bytes(down_rate_ema)}/s   ↑ {human_bytes(up_rate_ema)}/s   Total ↓ {human_bytes(net_now.bytes_recv)} ↑ {human_bytes(net_now.bytes_sent)}",
            width,
        )

        core_line = " ".join(f"C{i}:{v:4.0f}%" for i, v in enumerate(cpu_per_core))
        safe_addnstr(stdscr, 10, 0, f"Cores    : {core_line}", width)

        safe_addnstr(
            stdscr,
            12,
            0,
            f"Processes: {proc_total} total | Showing {len(procs)} by {'CPU' if sort_key == 'cpu' else 'MEM' if sort_key == 'mem' else 'PWR*'}"
            f"{' | /' + search_query if search_query else ''}"
            f"{' | LOCK PID ' + str(locked_pid) if locked_pid is not None else ''}",
            width,
            curses.A_BOLD,
        )
        safe_addnstr(stdscr, 13, 0, "PID      USER            CPU%   MEM%     PWR   NAME", width, curses.A_UNDERLINE)

        if procs:
            selected_index = max(0, min(selected_index, len(procs) - 1))
        else:
            selected_index = 0

        if selected_index < scroll_offset:
            scroll_offset = selected_index
        elif selected_index >= scroll_offset + visible_rows:
            scroll_offset = selected_index - visible_rows + 1

        max_scroll = max(0, len(procs) - visible_rows)
        scroll_offset = max(0, min(scroll_offset, max_scroll))
        visible_procs = procs[scroll_offset : scroll_offset + visible_rows]

        row = 14
        for idx, p in enumerate(visible_procs):
            if row >= height - 2:
                break
            absolute_index = scroll_offset + idx
            pid = p.get("pid", 0)
            user = (p.get("username") or "-")[:14]
            cpu = p.get("cpu_percent") or 0.0
            mem = p.get("memory_percent") or 0.0
            power = format_power_usage(p.get("power_watts"))
            name = (p.get("name") or "-")[: max(1, width - 46)]
            line = f"{pid:<8} {user:<14} {cpu:>5.1f}  {mem:>5.1f}  {power:>6}  {name}"
            row_attr = curses.A_REVERSE if absolute_index == selected_index else 0
            safe_addnstr(stdscr, row, 0, line, width, row_attr)
            row += 1

        bottom_line = ""
        if search_mode:
            blink_cursor = "|" if int(time.time() * 2) % 2 == 0 else " "
            bottom_line = f"Search   : {search_input}{blink_cursor}"
        elif search_query:
            bottom_line = f"Filter   : {search_query}  (press '/' to edit, Enter on empty to clear)"
        elif status_message and time.time() < status_until:
            bottom_line = f"Status   : {status_message}"
        else:
            status_message = ""

        safe_addnstr(stdscr, height - 2, 0, bottom_line.ljust(width), width)

        help_line = "q:quit  /:search  Enter:lock/unlock  ↑/↓ or j/k:move  PgUp/PgDn:page  Home/End  K:kill  c/m/p:sort  r:refresh"
        if max_scroll > 0:
            pos = min(len(procs), scroll_offset + 1)
            end = min(len(procs), scroll_offset + len(visible_procs))
            scroll_info = f"  [{pos}-{end}/{len(procs)}]"
            help_line = help_line + scroll_info
        safe_addnstr(stdscr, height - 1, 0, help_line.ljust(width), width, curses.A_REVERSE)
        stdscr.noutrefresh()
        curses.doupdate()


def main():
    parser = argparse.ArgumentParser(description="SysWatcher - terminal system monitor")
    parser.add_argument("-r", "--refresh", type=float, default=1.0, help="Refresh interval in seconds")
    parser.add_argument("-n", "--top", type=int, default=0, help="Number of processes to show (0 = all)")
    args = parser.parse_args()

    refresh_rate = max(0.2, args.refresh)
    proc_count = 0 if args.top <= 0 else max(5, args.top)

    try:
        curses.wrapper(draw, refresh_rate, proc_count)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
