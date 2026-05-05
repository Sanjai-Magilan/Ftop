use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemError {
    NotFound(String),
    PermissionDenied(String),
    #[cfg_attr(not(test), allow(dead_code))]
    InvalidData(String),
    Io(String),
}

impl fmt::Display for SystemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SystemError::NotFound(path) => write!(f, "not found: {}", path),
            SystemError::PermissionDenied(msg) => write!(f, "permission denied: {}", msg),
            SystemError::InvalidData(msg) => write!(f, "invalid data: {}", msg),
            SystemError::Io(msg) => write!(f, "io error: {}", msg),
        }
    }
}

impl std::error::Error for SystemError {}

pub trait SystemInterface {
    fn read_file(&self, path: &str) -> Result<String, SystemError>;
    fn read_dir(&self, path: &str) -> Result<Vec<String>, SystemError>;
    fn send_signal(&self, pid: i32, signal: Signal) -> Result<(), SystemError>;

    fn kill_process(&self, pid: i32) -> Result<(), SystemError> {
        self.send_signal(pid, Signal::SIGKILL)
    }
}

#[derive(Default)]
pub struct RealSystem;

impl SystemInterface for RealSystem {
    fn read_file(&self, path: &str) -> Result<String, SystemError> {
        fs::read_to_string(path).map_err(|err| map_io_error(path, err))
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, SystemError> {
        let entries = fs::read_dir(path).map_err(|err| map_io_error(path, err))?;
        let mut names = Vec::new();
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        Ok(names)
    }

    fn send_signal(&self, pid: i32, signal: Signal) -> Result<(), SystemError> {
        kill(Pid::from_raw(pid), signal).map_err(|err| match err {
            nix::Error::EPERM => SystemError::PermissionDenied(format!("pid {}", pid)),
            nix::Error::ESRCH => SystemError::NotFound(format!("pid {}", pid)),
            other => SystemError::Io(other.to_string()),
        })
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Default)]
pub struct MockSystem {
    files: RefCell<HashMap<String, Result<String, SystemError>>>,
    dirs: RefCell<HashMap<String, Result<Vec<String>, SystemError>>>,
    signals: RefCell<HashMap<(i32, i32), Result<(), SystemError>>>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl MockSystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_file(self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.files
            .borrow_mut()
            .insert(path.into(), Ok(content.into()));
        self
    }

    pub fn with_file_error(self, path: impl Into<String>, error: SystemError) -> Self {
        self.files.borrow_mut().insert(path.into(), Err(error));
        self
    }

    pub fn with_dir(self, path: impl Into<String>, entries: Vec<String>) -> Self {
        self.dirs.borrow_mut().insert(path.into(), Ok(entries));
        self
    }

    pub fn with_dir_error(self, path: impl Into<String>, error: SystemError) -> Self {
        self.dirs.borrow_mut().insert(path.into(), Err(error));
        self
    }

    pub fn with_signal_result(
        self,
        pid: i32,
        signal: Signal,
        result: Result<(), SystemError>,
    ) -> Self {
        self.signals
            .borrow_mut()
            .insert((pid, signal as i32), result);
        self
    }
}

impl SystemInterface for MockSystem {
    fn read_file(&self, path: &str) -> Result<String, SystemError> {
        match self.files.borrow().get(path) {
            Some(result) => result.clone(),
            None => Err(SystemError::NotFound(path.to_string())),
        }
    }

    fn read_dir(&self, path: &str) -> Result<Vec<String>, SystemError> {
        match self.dirs.borrow().get(path) {
            Some(result) => result.clone(),
            None => Err(SystemError::NotFound(path.to_string())),
        }
    }

    fn send_signal(&self, pid: i32, signal: Signal) -> Result<(), SystemError> {
        match self.signals.borrow().get(&(pid, signal as i32)) {
            Some(result) => result.clone(),
            None => Ok(()),
        }
    }
}

fn map_io_error(path: &str, err: io::Error) -> SystemError {
    match err.kind() {
        io::ErrorKind::NotFound => SystemError::NotFound(path.to_string()),
        io::ErrorKind::PermissionDenied => SystemError::PermissionDenied(path.to_string()),
        _ => SystemError::Io(err.to_string()),
    }
}
