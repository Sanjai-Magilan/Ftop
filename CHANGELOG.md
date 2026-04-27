# Changelog

All notable changes in this project from the beginning.

## 2026-04-27

- Fixed search cancel behavior so a single `Esc` press exits search mode reliably (no double-tap required).
- Improved keyboard handling for escape-sequence timing to make standalone `Esc` more responsive.
- Added a real terminal cursor in search mode and positioned it at the end of the search input.
- Updated search UX to use the terminal's high-visibility cursor style when supported.

## 2026-04-26

- Updated terminal UI layout so the search/filter/status bar appears at the bottom (second-last line).
- Kept shortcuts/help text on the last line.
- Adjusted process table viewport to reserve space for the bottom search/status bar.
- Updated README controls and UI behavior documentation.

## 2026-04-04 21:26:25 +0530

- Commit: `ceacd8d`
- Added initial system monitoring script to track CPU, memory, and disk usage.

## 2026-04-24 19:33:35 +0530

- Commit: `8f19d5f`
- Implemented terminal-based monitoring display.
- Added argument parsing.
- Added process sorting support.

## 2026-04-24 21:10:27 +0530

- Commit: `53e5504`
- Added README with project description, requirements, installation, and usage.

## 2026-04-26 13:03:08 +0530

- Commit: `11d0a12`
- Added power sorting option for top processes in the monitoring display (`p` key).

## 2026-04-26 13:08:14 +0530

- Commit: `2cbfd73`
- Updated README to include power sorting option for top processes.

## 2026-04-26 13:16:41 +0530

- Commit: `5be3e30`
- Updated documentation for power sorting and per-process power estimation behavior.

## 2026-04-26 13:43:04 +0530

- Commit: `840d4d6`
- Updated documentation and process display behavior for power sorting and full process listing.

## 2026-04-26 14:36:04 +0530

- Commit: `8d65835`
- Improved metric sampling (CPU, memory, network) and monitoring accuracy behavior.

## 2026-04-26 14:39:15 +0530

- Commit: `9b986f5`
- Removed color handling logic from the monitoring display.

## 2026-04-26 15:19:16 +0530

- Commit: `d0f653e`
- Improved memory usage sampling accuracy in the monitor.

## 2026-04-26 17:23:48 +0530

- Commit: `fabf7bf`
- Added process tree termination for kill action.
- Added process search/filter by name or PID.
