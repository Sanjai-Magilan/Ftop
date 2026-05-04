# SysWatcher

A lightweight, terminal-based system monitor written in Rust.

It shows live:

- CPU usage (total + per core)
- Load average
- Uptime
- Memory, swap, and disk usage
- Network up/down rate
- Process list (sorted by CPU, memory, or power)
- Bottom search/status bar for process filtering and action feedback

## Requirements

- Rust toolchain (stable)
- Linux terminal with UTF-8 support

## Build

```bash
cargo build --release
```

## Run

From the project directory:

```bash
cargo run --release
```

### Optional flags

```bash
cargo run --release -- --refresh 0.5 --top 20
```

- `--refresh`: refresh interval in seconds (minimum 0.2)
- `--top`: number of processes to display (`0` = all, default)

## Controls

- `q` → Quit
- `/` → Search/filter processes by name or PID
- `t` → Toggle process tree view
- `s` → Suspend (sleep) selected process
- `w` → Resume (wake) selected process
- `Enter` → Lock/unlock selection on the highlighted process
- `↑/↓` or `j/k` → Move selection
- `PgUp/PgDn` → Page up/down
- `Home/End` → Jump to top/bottom
- `x` → Kill selected process tree (process + children)
- `c` → Sort process list by CPU
- `m` → Sort process list by memory
- `p` → Sort process list by power consumption
- `r` → Refresh immediately

### Process control (suspend/resume)

- Press `s` to suspend (sleep) the selected process using SIGSTOP. The process will stop executing and enter a stopped state.
- Press `w` to resume (wake) a suspended process using SIGCONT. If the process is already running, this has no effect.
- Suspended processes remain in memory and can be resumed at any time.
- You must have permission (same user or root) to suspend/resume a process.
- Use `Enter` to lock the selection on a specific process to keep controlling it even as the list updates.
- **Selected process state**: The process panel header shows the selected process PID and its current state (RUNNING or STOPPED) for easy visibility.

### Process tree view

- Press `t` to switch the process panel between a flat list and a parent/child tree.
- Tree mode keeps the same sort key for sibling ordering and shows lineage using indentation and branch markers.
- Search still filters by process name or PID.

### Search bar behavior

- The search/filter input appears on the second-last line (bottom bar).
- The last line remains the help/shortcut line.
- Active filter text and status messages are shown in the same bottom bar.
- Press `/` to enter search mode.
- Press `Esc` once to cancel search mode.
- A blinking block cursor is shown while typing in search mode.

### Power column note

- `PWR` / `PWR*` is an estimated per-process power value (derived from CPU usage).
- If the estimated value is too low or cannot be calculated, the table shows `-`.

## Troubleshooting

### Terminal too small

- Resize your terminal to a larger size.
- Minimum recommended size: 40x8.

### Build errors

- Ensure Rust is installed: `rustc --version` and `cargo --version`.

## Project Structure

- `src/main.rs` — main terminal monitor app
- `Cargo.toml` — Rust dependencies and package metadata
- `Cargo.lock` — dependency lockfile for reproducible builds
- `.gitignore` — ignores Rust build output (`target/`) and editor temp files
