# Changelog

All notable changes in this project from the beginning.

## 2026-05-05 (continued)

- Reorganized test suite for production-scale clarity:
  - Created `src/lib.rs` to expose `parsers`, `process_controller`, and `system_interface` modules for library consumers.
  - Moved integration tests (6 tests) from source files to dedicated `tests/integration_test.rs` binary.
  - Kept unit tests (19 tests) in source modules for fast, localized validation.
  - Updated `Cargo.toml` to define both `[lib]` and `[[bin]]` sections.
  - Total: 30 passing tests verifying end-to-end metrics collection, MockSystem workflows, and OS abstraction.

## 2026-05-05

- Updated kill confirmation behavior to show a child-process specific warning message: "this is a child process do u want to kill it".

## 2026-05-04

- Added a process tree view toggle in the process panel.
- Rendered process hierarchy with parent/child branch markers while keeping sibling ordering by the active sort key.
- Updated memory display values to show two decimal places.

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
- Reimplemented the terminal monitor loop, live metrics, process controls, and process-tree kill behavior in Rust.
- Updated README, `.gitignore`, and search UX for the new Rust workflow, including a reliable `Esc` cancel path and visible search cursor.
- Improved Linux metric accuracy and robustness for memory, network, and disk sampling.
- Removed legacy Python source and cache files.

## 2026-04-26

- Updated terminal UI layout so the search/filter/status bar appears at the bottom (second-last line).
- Kept shortcuts/help text on the last line.
- Adjusted process table viewport to reserve space for the bottom search/status bar.
- Updated README controls and UI behavior documentation.

- Added power sorting for top processes (`p` key) and documented per-process power estimation.
- Improved metric sampling accuracy, removed color handling from the monitoring display, and refined memory usage calculations.
- Added process-tree termination for kill actions and process search/filter by name or PID.

## 2026-04-24

- Implemented terminal-based monitoring display.
- Added argument parsing.
- Added process sorting support.
- Added README with project description, requirements, installation, and usage.

## 2026-04-04

- Added initial system monitoring script to track CPU, memory, and disk usage.
