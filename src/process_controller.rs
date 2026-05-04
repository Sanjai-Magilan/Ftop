use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fs;

/// Custom error type for process control operations.
#[derive(Debug, Clone)]
pub enum ProcessError {
    PermissionDenied(i32),
    ProcessNotFound(i32),
    SignalError(String),
    StatusReadError(String),
    InvalidStatus(String),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessError::PermissionDenied(pid) => write!(f, "Permission denied for PID {}", pid),
            ProcessError::ProcessNotFound(pid) => write!(f, "Process {} not found", pid),
            ProcessError::SignalError(msg) => write!(f, "Signal error: {}", msg),
            ProcessError::StatusReadError(msg) => write!(f, "Error reading status: {}", msg),
            ProcessError::InvalidStatus(msg) => write!(f, "Invalid status: {}", msg),
        }
    }
}

impl std::error::Error for ProcessError {}

/// ProcessController manages process state transitions (suspend/resume).
///
/// This controller provides thread-safe operations to control process execution.
/// It can be safely called from multiple threads without synchronization overhead.
pub struct ProcessController;

impl ProcessController {
    /// Suspend a process by sending SIGSTOP.
    ///
    /// SIGSTOP suspends process execution immediately and cannot be caught or ignored.
    /// The process enters the 'T' (Stopped) state and will not resume until SIGCONT is sent.
    /// This is a hard stop that is reliable for controlling any process you have permission to control.
    ///
    /// # Arguments
    /// * `pid` - The process ID to suspend
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(ProcessError)` on failure (permission denied, process not found, etc.)
    ///
    /// # Example
    /// ```no_run
    /// use syswatcher::process_controller::ProcessController;
    /// let pid = 12345;
    /// match ProcessController::suspend_process(pid) {
    ///     Ok(()) => println!("Process {} suspended", pid),
    ///     Err(e) => eprintln!("Failed to suspend: {}", e),
    /// }
    /// ```
    pub fn suspend_process(pid: i32) -> Result<(), ProcessError> {
        Self::send_signal(pid, Signal::SIGSTOP)
    }

    /// Resume a process by sending SIGCONT.
    ///
    /// SIGCONT resumes execution of a stopped process. If the process is not stopped,
    /// this signal has no effect but still succeeds. This allows safe resumption without
    /// needing to check the current state first.
    ///
    /// # Arguments
    /// * `pid` - The process ID to resume
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(ProcessError)` on failure (permission denied, process not found, etc.)
    ///
    /// # Example
    /// ```no_run
    /// use syswatcher::process_controller::ProcessController;
    /// let pid = 12345;
    /// match ProcessController::resume_process(pid) {
    ///     Ok(()) => println!("Process {} resumed", pid),
    ///     Err(e) => eprintln!("Failed to resume: {}", e),
    /// }
    /// ```
    pub fn resume_process(pid: i32) -> Result<(), ProcessError> {
        Self::send_signal(pid, Signal::SIGCONT)
    }

    /// Check if a process is in the 'Stopped' (T) state.
    ///
    /// Reads the process status from `/proc/[pid]/status` and parses the State field.
    /// A process is considered stopped if its state is 'T' (traced/stopped).
    /// This is useful for UI state display and conditional resume logic.
    ///
    /// # Arguments
    /// * `pid` - The process ID to check
    ///
    /// # Returns
    /// * `Ok(true)` if the process is in 'T' (stopped) state
    /// * `Ok(false)` if the process is in any other state
    /// * `Err(ProcessError)` if the process doesn't exist or cannot be read
    ///
    /// # Example
    /// ```no_run
    /// use syswatcher::process_controller::ProcessController;
    /// let pid = 12345;
    /// match ProcessController::get_process_status(pid) {
    ///     Ok(true) => println!("Process {} is stopped", pid),
    ///     Ok(false) => println!("Process {} is running", pid),
    ///     Err(e) => eprintln!("Error: {}", e),
    /// }
    /// ```
    pub fn get_process_status(pid: i32) -> Result<bool, ProcessError> {
        let status_path = format!("/proc/{}/status", pid);

        let content = fs::read_to_string(&status_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProcessError::ProcessNotFound(pid)
            } else {
                ProcessError::StatusReadError(e.to_string())
            }
        })?;

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
            "State field not found in /proc/[pid]/status".to_string(),
        ))
    }

    /// Internal helper to send a signal to a process.
    /// Maps nix errors to our ProcessError enum for consistent error reporting.
    fn send_signal(pid: i32, signal: Signal) -> Result<(), ProcessError> {
        kill(Pid::from_raw(pid), signal).map_err(|e| match e {
            nix::Error::EPERM => ProcessError::PermissionDenied(pid),
            nix::Error::ESRCH => ProcessError::ProcessNotFound(pid),
            _ => ProcessError::SignalError(e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_pid_error() {
        // Test with an obviously invalid PID
        let result = ProcessController::get_process_status(-1);
        assert!(result.is_err());
    }

    #[test]
    fn test_nonexistent_pid() {
        // Use a very high PID that shouldn't exist
        let result = ProcessController::get_process_status(999999);
        assert!(matches!(result, Err(ProcessError::ProcessNotFound(_))));
    }
}
