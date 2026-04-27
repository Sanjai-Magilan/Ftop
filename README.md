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
- `Enter` → Lock/unlock selection on the highlighted process
- `↑/↓` or `j/k` → Move selection
- `PgUp/PgDn` → Page up/down
- `Home/End` → Jump to top/bottom
- `x` → Kill selected process tree (process + children)
- `c` → Sort process list by CPU
- `m` → Sort process list by memory
- `p` → Sort process list by power consumption
- `r` → Refresh immediately

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
