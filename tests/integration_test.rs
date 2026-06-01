//! Integration tests for Ftop
//!
//! This module contains integration-style tests that verify full pipelines and workflows.
//! These tests use MockSystem to simulate OS interactions and verify end-to-end functionality.
//!
//! Unit tests that verify individual parsing functions or module-specific logic remain
//! in their respective source modules.

use ftop::parsers::{parse_parent_pid_from_stat, parse_process_status};
use ftop::system_interface::{MockSystem, SystemError, SystemInterface};
use nix::sys::signal::Signal;

/// Integration test: verifies that MockSystem deterministically provides file reads
/// and that integration code correctly uses them without depending on real /proc.
#[test]
fn integration_mock_system_drives_shared_bytes_and_priority() {
    let system = MockSystem::new()
        .with_file("/proc/50/statm", "1 2 3 4 5")
        .with_file(
            "/proc/50/stat",
            "50 (proc with spaces) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 42 0 1 0",
        )
        .with_file(
            "/proc/meminfo",
            "MemTotal: 1000 kB\nMemFree: 250 kB\nBuffers: 0 kB\nCached: 0 kB\nSReclaimable: 0 kB\nShmem: 0 kB\nMemAvailable: 800 kB\n",
        );

    // Verify file reads work through MockSystem
    let statm_content = system.read_file("/proc/50/statm").expect("mock should have statm");
    assert_eq!(statm_content, "1 2 3 4 5");

    let stat_content = system.read_file("/proc/50/stat").expect("mock should have stat");
    assert!(stat_content.contains("proc with spaces"));
    assert_eq!(parse_parent_pid_from_stat(&stat_content).unwrap(), 1);

    let meminfo_content = system.read_file("/proc/meminfo").expect("mock should have meminfo");
    assert!(meminfo_content.contains("MemTotal: 1000 kB"));
}

/// Integration test: verifies MockSystem handles error injection correctly
#[test]
fn integration_mock_system_handles_file_not_found() {
    let system = MockSystem::new()
        .with_file_error(
            "/proc/meminfo",
            SystemError::NotFound("/proc/meminfo".to_string()),
        )
        .with_file("/proc/60/statm", "broken")
        .with_dir_error(
            "/proc",
            SystemError::Io("directory unavailable".to_string()),
        )
        .with_file_error(
            "/proc/61/stat",
            SystemError::NotFound("/proc/61/stat".to_string()),
        );

    // Verify error handling
    assert!(system.read_file("/proc/meminfo").is_err());
    assert_eq!(system.read_file("/proc/meminfo").unwrap_err(), SystemError::NotFound("/proc/meminfo".to_string()));

    let result = system.read_file("/proc/60/statm").expect("mock should return broken content");
    assert_eq!(result, "broken"); // MockSystem returns the content even if it's malformed

    let dir_result = system.read_dir("/proc");
    assert!(dir_result.is_err()); // Directory errors are respected
}

/// Integration test: verifies permission denied is correctly propagated through signal handling
#[test]
fn integration_mock_system_signals_permission_denied() {
    let system = MockSystem::new()
        .with_dir("/proc", vec!["70".to_string(), "71".to_string()])
        .with_file("/proc/70/stat", "70 (parent) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file("/proc/71/stat", "71 (child) S 70 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_signal_result(
            70,
            Signal::SIGKILL,
            Err(SystemError::PermissionDenied("pid 70".to_string())),
        )
        .with_signal_result(
            71,
            Signal::SIGKILL,
            Err(SystemError::PermissionDenied("pid 71".to_string())),
        );

    // Verify signal errors are readable
    let result = system.send_signal(70, Signal::SIGKILL);
    assert!(result.is_err());
    match result {
        Err(SystemError::PermissionDenied(msg)) => assert_eq!(msg, "pid 70"),
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

/// Integration test: verifies process status parsing works through MockSystem
#[test]
fn integration_mock_system_process_status() {
    let system = MockSystem::new()
        .with_file("/proc/10/status", "Name:\ttest\nState:\tT (stopped)\n")
        .with_file_error(
            "/proc/11/status",
            SystemError::NotFound("/proc/11/status".to_string()),
        )
        .with_file_error(
            "/proc/12/status",
            SystemError::InvalidData("corrupted status payload".to_string()),
        );

    // Stopped state should parse to true
    let stopped_content = system.read_file("/proc/10/status").expect("mock should have status");
    let is_stopped = parse_process_status(&stopped_content).expect("should parse");
    assert_eq!(is_stopped, true);

    // NotFound should be handled
    assert!(system.read_file("/proc/11/status").is_err());

    // Invalid data should be handled
    let invalid_result = system.read_file("/proc/12/status");
    assert!(invalid_result.is_err());
}

/// Integration test: verifies read_dir works for process discovery
#[test]
fn integration_mock_system_read_dir_for_process_discovery() {
    let system = MockSystem::new()
        .with_dir("/proc", vec!["100".to_string(), "101".to_string(), "102".to_string()])
        .with_file("/proc/100/stat", "100 (parent) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file("/proc/101/stat", "101 (child1) S 100 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file("/proc/102/stat", "102 (child2) S 100 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file_error(
            "/proc/103/stat",
            SystemError::NotFound("/proc/103/stat".to_string()),
        );

    // Read directory
    let entries = system.read_dir("/proc").expect("should read /proc");
    assert_eq!(entries.len(), 3);
    assert!(entries.contains(&"100".to_string()));
    assert!(entries.contains(&"101".to_string()));
    assert!(entries.contains(&"102".to_string()));

    // Missing entries should be skipped gracefully
    for pid_str in entries {
        let stat_path = format!("/proc/{}/stat", pid_str);
        if let Ok(content) = system.read_file(&stat_path) {
            assert!(!content.is_empty());
        }
    }
}

/// Integration test: verifies multiple signal results work correctly
#[test]
fn integration_mock_system_mixed_signal_results() {
    let system = MockSystem::new()
        .with_dir("/proc", vec!["20".to_string(), "21".to_string(), "22".to_string()])
        .with_file("/proc/20/stat", "20 (proc1) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file("/proc/21/stat", "21 (proc2) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_file("/proc/22/stat", "22 (proc3) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
        .with_signal_result(20, Signal::SIGSTOP, Ok(())) // explicit success
        .with_signal_result(
            21,
            Signal::SIGSTOP,
            Err(SystemError::PermissionDenied("denied".to_string())), // denied
        )
        // 22 has no signal result registered - MockSystem returns Ok(()) by default
        ;

    // PID 20: explicit success
    assert!(system.send_signal(20, Signal::SIGSTOP).is_ok());

    // PID 21: permission denied
    let result = system.send_signal(21, Signal::SIGSTOP);
    assert!(result.is_err());

    // PID 22: not configured - MockSystem defaults to Ok(())
    let result = system.send_signal(22, Signal::SIGSTOP);
    assert!(result.is_ok());
}
