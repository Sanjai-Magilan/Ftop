# SysWatcher

A lightweight, terminal-based system monitor written in Python.

It shows live:

- CPU usage (total + per core)
- Load average
- Uptime
- Memory, swap, and disk usage
- Network up/down rate
- Top processes (sorted by CPU or memory)

## Requirements

- Python 3.8+
- `psutil`
- A terminal that supports `curses` (Linux/macOS terminal)

## Installation

```bash
python3 -m pip install psutil
```

## Run

From the project directory:

```bash
python3 monitor.py
```

### Optional flags

```bash
python3 monitor.py --refresh 0.5 --top 20
```

- `--refresh`: refresh interval in seconds (minimum 0.2)
- `--top`: number of top processes to display (minimum 5)

## Controls

- `q` → Quit
- `c` → Sort process list by CPU
- `m` → Sort process list by memory
- `r` → Refresh immediately

## Troubleshooting

### `_curses.error: addnwstr() returned ERR`

- Resize your terminal to a larger size.
- The app now handles small terminal sizes gracefully and shows a warning.

### Module not found: `psutil`

Install dependency:

```bash
python3 -m pip install psutil
```

## Project Structure

- `monitor.py` — main terminal monitor app
