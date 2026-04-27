# Changelog

All notable changes in this project from the beginning.

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
