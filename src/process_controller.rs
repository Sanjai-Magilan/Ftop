use crate::system_interface::{RealSystem, SystemError, SystemInterface};
use nix::sys::signal::Signal;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum ProcessError {
    PermissionDenied(i32),
    ProcessNotFound(i32),
    StatusReadError(String),
    InvalidStatus(String),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::PermissionDenied(pid) => write!(f, "Permission denied for PID {}", pid),
            ProcessError::ProcessNotFound(pid) => write!(f, "Process {} not found", pid),
            ProcessError::StatusReadError(msg) => write!(f, "Error reading status: {}", msg),
            ProcessError::InvalidStatus(msg) => write!(f, "Invalid status: {}", msg),
        }
    }
}

impl std::error::Error for ProcessError {}

pub struct ProcessController;

impl ProcessController {
    pub fn suspend_process(pid: i32) -> Result<(), ProcessError> {
        let system = RealSystem::default();
        Self::send_signal_tree_with(&system, pid, Signal::SIGSTOP)
    }

    pub fn resume_process(pid: i32) -> Result<(), ProcessError> {
        let system = RealSystem::default();
        Self::send_signal_tree_with(&system, pid, Signal::SIGCONT)
    }

    #[allow(dead_code)]
    pub fn suspend_process_with<S: SystemInterface + ?Sized>(
        system: &S,
        pid: i32,
    ) -> Result<(), ProcessError> {
        Self::send_signal_tree_with(system, pid, Signal::SIGSTOP)
    }

    #[allow(dead_code)]
    pub fn resume_process_with<S: SystemInterface + ?Sized>(
        system: &S,
        pid: i32,
    ) -> Result<(), ProcessError> {
        Self::send_signal_tree_with(system, pid, Signal::SIGCONT)
    }

    pub fn get_process_status(pid: i32) -> Result<bool, ProcessError> {
        let system = RealSystem::default();
        Self::get_process_status_with(&system, pid)
    }

    pub fn get_process_status_with<S: SystemInterface + ?Sized>(
        system: &S,
        pid: i32,
    ) -> Result<bool, ProcessError> {
        let status_path = format!("/proc/{}/status", pid);
        let content = system.read_file(&status_path).map_err(|err| map_system_error(pid, err))?;
        Self::parse_process_status(&content)
    }

    pub fn parse_process_status(content: &str) -> Result<bool, ProcessError> {
        for line in content.lines() {
            if line.starts_with("State:") {
                let state = line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| ProcessError::InvalidStatus("Could not parse state".to_string()))?;
                return Ok(state == "T");
            }
        }

        Err(ProcessError::InvalidStatus(
            "State field not found in status content".to_string(),
        ))
    }

    fn send_signal_with<S: SystemInterface + ?Sized>(
        system: &S,
        pid: i32,
        signal: Signal,
    ) -> Result<(), ProcessError> {
        system.send_signal(pid, signal).map_err(|err| map_system_error(pid, err))
    }

    pub fn send_signal_tree_with<S: SystemInterface + ?Sized>(
        system: &S,
        pid: i32,
        signal: Signal,
    ) -> Result<(), ProcessError> {
        if pid <= 0 {
            return Err(ProcessError::InvalidStatus("Invalid process id".to_string()));
        }

        let ppid_map = read_ppid_map_with(system);
        let mut targets = Vec::new();
        collect_descendants(pid, &ppid_map, &mut targets);
        targets.push(pid);

        let mut signaled = 0_u32;
        let mut denied = 0_u32;

        for target in targets {
            match Self::send_signal_with(system, target, signal) {
                Ok(()) => signaled += 1,
                Err(ProcessError::PermissionDenied(_)) => denied += 1,
                Err(ProcessError::ProcessNotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }

        if signaled == 0 {
            if denied > 0 {
                return Err(ProcessError::PermissionDenied(pid));
            }
            return Err(ProcessError::ProcessNotFound(pid));
        }

        if denied > 0 {
            return Err(ProcessError::PermissionDenied(pid));
        }

        Ok(())
    }
}

fn map_system_error(pid: i32, error: SystemError) -> ProcessError {
    match error {
        SystemError::NotFound(_) => ProcessError::ProcessNotFound(pid),
        SystemError::PermissionDenied(_) => ProcessError::PermissionDenied(pid),
        SystemError::InvalidData(message) => ProcessError::InvalidStatus(message),
        SystemError::Io(message) => ProcessError::StatusReadError(message),
    }
}

#[allow(dead_code)]
fn read_ppid_map() -> HashMap<i32, Vec<i32>> {
    let system = RealSystem::default();
    read_ppid_map_with(&system)
}

pub fn read_ppid_map_with<S: SystemInterface + ?Sized>(system: &S) -> HashMap<i32, Vec<i32>> {
    let mut map: HashMap<i32, Vec<i32>> = HashMap::new();

    let Ok(entries) = system.read_dir("/proc") else {
        return map;
    };

    for file_name in entries {
        let Ok(pid) = file_name.parse::<i32>() else {
            continue;
        };

        let stat_path = format!("/proc/{}/stat", pid);
        let Ok(stat) = system.read_file(&stat_path) else {
            continue;
        };
        let Some(ppid) = parse_parent_pid_from_stat(&stat) else {
            continue;
        };

        map.entry(ppid).or_default().push(pid);
    }

    map
}

fn collect_descendants(pid: i32, map: &HashMap<i32, Vec<i32>>, out: &mut Vec<i32>) {
    if let Some(children) = map.get(&pid) {
        for child in children {
            collect_descendants(*child, map, out);
            out.push(*child);
        }
    }
}

fn read_parent_pid(pid: i32) -> Option<i32> {
    let system = RealSystem::default();
    read_parent_pid_with(&system, pid)
}

pub fn read_parent_pid_with<S: SystemInterface + ?Sized>(system: &S, pid: i32) -> Option<i32> {
    if pid <= 0 {
        return None;
    }

    let stat_path = format!("/proc/{}/stat", pid);
    let stat = system.read_file(&stat_path).ok()?;
    parse_parent_pid_from_stat(&stat)
}

pub fn parse_parent_pid_from_stat(content: &str) -> Option<i32> {
    let end_comm = content.rfind(") ")?;
    let tail = &content[end_comm + 2..];
    let fields = tail.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 2 {
        return None;
    }
    fields[1].parse::<i32>().ok()
}

fn find_stopped_ancestor(pid: i32) -> Option<i32> {
    let system = RealSystem::default();
    find_stopped_ancestor_with(&system, pid)
}

pub fn find_stopped_ancestor_with<S: SystemInterface + ?Sized>(system: &S, pid: i32) -> Option<i32> {
    let mut current = pid;
    for _ in 0..128 {
        let parent_pid = read_parent_pid_with(system, current)?;
        if parent_pid <= 1 || parent_pid == current {
            return None;
        }

        if let Ok(true) = ProcessController::get_process_status_with(system, parent_pid) {
            return Some(parent_pid);
        }

        current = parent_pid;
    }

    None
}

pub fn suspend_conflict(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }

    let ppid_map = read_ppid_map();
    let mut descendants = Vec::new();
    collect_descendants(pid, &ppid_map, &mut descendants);

    if let Some(parent) = read_parent_pid(pid) {
        if parent > 0 {
            if !descendants.is_empty() {
                return Some(format!(
                    "PID {} has {} child(ren) and parent PID {} exists; suspend will apply to subtree",
                    pid,
                    descendants.len(),
                    parent
                ));
            }

            return Some(format!(
                "PID {} has parent PID {}; suspending the child without parent may be unintended",
                pid, parent
            ));
        }
    }

    if !descendants.is_empty() {
        Some(format!(
            "PID {} has {} child(ren); suspend will affect subtree",
            pid,
            descendants.len()
        ))
    } else {
        None
    }
}

pub fn resume_conflict(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }

    let mut parts = Vec::new();
    if let Some(ancestor) = find_stopped_ancestor(pid) {
        parts.push(format!("ancestor PID {} is stopped", ancestor));
    }

    let ppid_map = read_ppid_map();
    let mut descendants = Vec::new();
    collect_descendants(pid, &ppid_map, &mut descendants);
    if !descendants.is_empty() {
        parts.push(format!("resume will apply to {} descendant(s)", descendants.len()));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_interface::{MockSystem, SystemError};
    use nix::sys::signal::Signal;

    #[test]
    fn test_invalid_pid_error() {
        let result = ProcessController::get_process_status(-1);
        assert!(result.is_err());
    }

    #[test]
    fn test_nonexistent_pid() {
        let result = ProcessController::get_process_status(999999);
        assert!(matches!(result, Err(ProcessError::ProcessNotFound(_))));
    }

    #[test]
    fn parse_process_status_parses_stopped_and_running() {
        let stopped = "Name:\ttest\nState:\tT (stopped)\n";
        let running = "Name:\ttest\nState:\tR (running)\n";

        assert!(ProcessController::parse_process_status(stopped).unwrap());
        assert!(!ProcessController::parse_process_status(running).unwrap());
    }

    #[test]
    fn parse_process_status_handles_invalid_input() {
        let bad = "Name: foo\n";
        match ProcessController::parse_process_status(bad) {
            Err(ProcessError::InvalidStatus(_)) => {}
            other => panic!("expected InvalidStatus, got {:?}", other),
        }
    }

    #[test]
    fn parse_parent_pid_from_stat_handles_various_inputs() {
        let stat = "1234 (bash) S 42 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0";
        assert_eq!(parse_parent_pid_from_stat(stat), Some(42));
        assert_eq!(parse_parent_pid_from_stat(""), None);
        assert_eq!(parse_parent_pid_from_stat("1 (x)"), None);
    }

    #[test]
    fn mock_status_and_file_not_found_are_deterministic() {
        // Prevents flaky tests by avoiding live /proc reads.
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

        assert_eq!(ProcessController::get_process_status_with(&system, 10).unwrap(), true);
        assert!(matches!(
            ProcessController::get_process_status_with(&system, 11),
            Err(ProcessError::ProcessNotFound(11))
        ));
        assert!(matches!(
            ProcessController::get_process_status_with(&system, 12),
            Err(ProcessError::InvalidStatus(_))
        ));
    }

    #[test]
    fn mock_send_signal_reports_permission_denied() {
        // Prevents silently treating EPERM as success when the caller lacks privileges.
        let system = MockSystem::new()
            .with_dir("/proc", vec!["21".to_string()])
            .with_file("/proc/21/stat", "21 (worker) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
            .with_signal_result(
                21,
                Signal::SIGKILL,
                Err(SystemError::PermissionDenied("pid 21".to_string())),
            );

        let result = ProcessController::send_signal_tree_with(&system, 21, Signal::SIGKILL);
        assert!(matches!(result, Err(ProcessError::PermissionDenied(21))));
    }

    #[test]
    fn mock_ppid_map_skips_missing_stat_files() {
        // Prevents a disappearing process from crashing tree construction.
        let system = MockSystem::new()
            .with_dir("/proc", vec!["100".to_string(), "101".to_string()])
            .with_file("/proc/100/stat", "100 (parent) S 1 1 1 1 1 0 0 0 0 0 0 0 0 0 20 0 1 0")
            .with_file_error(
                "/proc/101/stat",
                SystemError::NotFound("/proc/101/stat".to_string()),
            );

        let map = read_ppid_map_with(&system);
        assert_eq!(map.get(&1).unwrap(), &vec![100]);
        assert!(!map.values().any(|children| children.contains(&101)));
    }
}
