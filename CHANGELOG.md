# Changelog

All notable changes in this project from the beginning.

## 2026-05-03

- Updated the terminal UI color palette with a softer dark theme and clearer accent colors.
- Fixed system meter bar alignment so Memory, Swap, and Disk bars end consistently.
- Migrated the terminal UI rendering layer to `ratatui` styles for a modern, minimal dashboard theme.
- Reduced dependency bloat by disabling `sysinfo`'s default multithreaded `rayon` feature.
- Reduced runtime overhead by redrawing the dashboard only when input, resize, status, or metrics change.
- Fixed CPU core display to show all available cores by dynamically sizing the CPU grid (up to 4 columns wide).
- Improved CPU panel layout to display cores in a clear 4×4 grid order: 0-3, 4-7, 8-11, 12-15.
- Adjusted top panel width allocation to give more space to the CPU section (80%) vs Memory/Disk/Net (20%).
- Made CPU section dimensions configurable: height (`top_h` on line 951), width (`Constraint::Percentage` on line 966), and progress bar size (line 643 `core_line` and line 28 `METER_STATIC_WIDTH`).

## 2026-05-02

- Added per-process runtime (uptime) to the process table.
- Added per-process memory detail columns: shared memory, virtual memory, and resident memory.
- Added per-process priority column sourced from Linux `/proc/<pid>/stat`.
- Updated process row formatting and column widths to improve table alignment.
- Updated process memory presentation formatting for clearer per-column measurements.
- Fixed narrow-terminal process table overflow by switching between full, compact, and tiny responsive layouts.

## 2026-04-27

- Migrated SysWatcher from Python to Rust.
- Added Rust project files: `Cargo.toml` and `src/main.rs`.
- Reimplemented terminal monitor loop, live metrics, process list sorting/filtering, lock, and process-tree kill in Rust.
- Updated README for Cargo build/run workflow.
- Fixed search cancel behavior so a single `Esc` press exits search mode reliably (no double-tap required).
- Improved keyboard handling for escape-sequence timing to make standalone `Esc` more responsive.
- Added a real terminal cursor in search mode and positioned it at the end of the search input.
- Updated search UX to use the terminal's high-visibility cursor style when supported.
- Updated `.gitignore` for Rust workflow (`target/` and editor temp files).
- Removed legacy Python source and cache files from the project.
- Improved memory measurement accuracy by parsing Linux `/proc/meminfo` fields including `SReclaimable`, `Shmem`, and `MemAvailable`.
- Hardened `/proc/meminfo` and `/proc/net/dev` parsing to prevent one malformed line from breaking all metrics.
- Fixed disk measurement accuracy by using direct `statvfs` syscall on root filesystem instead of relying on sysinfo crate.

## 2026-04-26

- Updated terminal UI layout so the search/filter/status bar appears at the bottom (second-last line).
- Kept shortcuts/help text on the last line.
- Adjusted process table viewport to reserve space for the bottom search/status bar.
- Updated README controls and UI behavior documentation.

- Added power sorting option for top processes in the monitoring display (`p` key).
- Updated README to include power sorting option for top processes.
- Updated documentation for power sorting and per-process power estimation behavior.
- Updated documentation and process display behavior for power sorting and full process listing.
- Improved metric sampling (CPU, memory, network) and monitoring accuracy behavior.
- Removed color handling logic from the monitoring display.
- Improved memory usage sampling accuracy in the monitor.
- Added process tree termination for kill action.
- Added process search/filter by name or PID.

## 2026-04-24

- Implemented terminal-based monitoring display.
- Added argument parsing.
- Added process sorting support.
- Added README with project description, requirements, installation, and usage.

## 2026-04-04

- Added initial system monitoring script to track CPU, memory, and disk usage.
