import argparse
import curses
import os
import socket
import time
from datetime import datetime
from typing import Optional

import psutil


ASSUMED_CPU_PACKAGE_POWER_W = 65.0
MIN_DISPLAY_POWER_W = 0.1


def human_bytes(value: float) -> str:
    units = ["B", "KB", "MB", "GB", "TB", "PB"]
    i = 0
    while value >= 1024 and i < len(units) - 1:
        value /= 1024
        i += 1
    return f"{value:.1f}{units[i]}"


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


def get_top_processes(limit: int, sort_key: str, logical_cpus: int):
    procs = []
    for proc in psutil.process_iter(["pid", "name", "username", "cpu_percent", "memory_percent"]):
        try:
            info = proc.info
            info["power_watts"] = estimate_process_power_watts(info.get("cpu_percent") or 0.0, logical_cpus)
            procs.append(info)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue

    if sort_key == "mem":
        procs.sort(key=lambda p: p.get("memory_percent") or 0.0, reverse=True)
    elif sort_key == "power":
        procs.sort(key=lambda p: p.get("power_watts") or 0.0, reverse=True)
    else:
        procs.sort(key=lambda p: p.get("cpu_percent") or 0.0, reverse=True)

    return procs[:limit]


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
    stdscr.timeout(int(refresh_rate * 1000))

    # Prime CPU counters so process CPU% values become meaningful on next updates.
    psutil.cpu_percent(interval=None)
    for proc in psutil.process_iter():
        try:
            proc.cpu_percent(interval=None)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue

    sort_key = "cpu"
    net_prev = psutil.net_io_counters()
    net_prev_time = time.time()
    logical_cpus = psutil.cpu_count(logical=True) or 1

    while True:
        height, width = stdscr.getmaxyx()
        stdscr.erase()

        if height < 8 or width < 40:
            safe_addnstr(stdscr, 0, 0, "Terminal too small. Resize window (min 40x8). Press q to quit.", width)
            stdscr.refresh()
            key = stdscr.getch()
            if key in (ord("q"), ord("Q")):
                break
            continue

        now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        host = socket.gethostname()

        cpu_total = psutil.cpu_percent(interval=None)
        cpu_per_core = psutil.cpu_percent(interval=None, percpu=True)
        load_avg = os.getloadavg() if hasattr(os, "getloadavg") else (0.0, 0.0, 0.0)

        vm = psutil.virtual_memory()
        sm = psutil.swap_memory()
        disk = psutil.disk_usage("/")
        boot_time = psutil.boot_time()
        uptime = human_uptime(time.time() - boot_time)

        net_now = psutil.net_io_counters()
        now_t = time.time()
        dt = max(0.001, now_t - net_prev_time)
        down_rate = (net_now.bytes_recv - net_prev.bytes_recv) / dt
        up_rate = (net_now.bytes_sent - net_prev.bytes_sent) / dt
        net_prev = net_now
        net_prev_time = now_t

        procs = get_top_processes(proc_count, sort_key, logical_cpus)
        proc_total = len(psutil.pids())

        bar_w = max(10, min(40, width - 38))

        title = f" SysWatcher • {host} • {now} "
        safe_addnstr(stdscr, 0, 0, title.ljust(width), width, curses.A_REVERSE)

        safe_addnstr(stdscr, 2, 0, f"CPU Total: {cpu_total:5.1f}%  [{progress_bar(cpu_total, bar_w)}]", width)
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
            f"Memory   : {vm.percent:5.1f}%  {human_bytes(vm.used)}/{human_bytes(vm.total)}  [{progress_bar(vm.percent, bar_w)}]",
            width,
        )
        safe_addnstr(
            stdscr,
            6,
            0,
            f"Swap     : {sm.percent:5.1f}%  {human_bytes(sm.used)}/{human_bytes(sm.total)}  [{progress_bar(sm.percent, bar_w)}]",
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
            f"Network  : ↓ {human_bytes(down_rate)}/s   ↑ {human_bytes(up_rate)}/s   Total ↓ {human_bytes(net_now.bytes_recv)} ↑ {human_bytes(net_now.bytes_sent)}",
            width,
        )

        core_line = " ".join(f"C{i}:{v:4.0f}%" for i, v in enumerate(cpu_per_core))
        safe_addnstr(stdscr, 10, 0, f"Cores    : {core_line}", width)

        safe_addnstr(
            stdscr,
            12,
            0,
            f"Processes: {proc_total} total | Top {len(procs)} by {'CPU' if sort_key == 'cpu' else 'MEM' if sort_key == 'mem' else 'PWR*'}",
            width,
            curses.A_BOLD,
        )
        safe_addnstr(stdscr, 13, 0, "PID      USER            CPU%   MEM%     PWR   NAME", width, curses.A_UNDERLINE)

        row = 14
        for p in procs:
            if row >= height - 2:
                break
            pid = p.get("pid", 0)
            user = (p.get("username") or "-")[:14]
            cpu = p.get("cpu_percent") or 0.0
            mem = p.get("memory_percent") or 0.0
            power = format_power_usage(p.get("power_watts"))
            name = (p.get("name") or "-")[: max(1, width - 46)]
            line = f"{pid:<8} {user:<14} {cpu:>5.1f}  {mem:>5.1f}  {power:>6}  {name}"
            safe_addnstr(stdscr, row, 0, line, width)
            row += 1

        help_line = "q:quit  c:sort CPU  m:sort MEM  p:sort PWR*  r:refresh"
        safe_addnstr(stdscr, height - 1, 0, help_line.ljust(width), width, curses.A_REVERSE)

        stdscr.refresh()

        key = stdscr.getch()
        if key in (ord("q"), ord("Q")):
            break
        if key in (ord("c"), ord("C")):
            sort_key = "cpu"
        elif key in (ord("m"), ord("M")):
            sort_key = "mem"
        elif key in (ord("p"), ord("P")):
            sort_key = "power"
        elif key in (ord("r"), ord("R")):
            pass


def main():
    parser = argparse.ArgumentParser(description="SysWatcher - terminal system monitor")
    parser.add_argument("-r", "--refresh", type=float, default=1.0, help="Refresh interval in seconds")
    parser.add_argument("-n", "--top", type=int, default=12, help="Number of top processes to show")
    args = parser.parse_args()

    refresh_rate = max(0.2, args.refresh)
    proc_count = max(5, args.top)

    try:
        curses.wrapper(draw, refresh_rate, proc_count)
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
