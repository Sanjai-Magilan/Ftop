//! SysWatcher - A high-performance system monitoring library
//!
//! This crate provides core functionality for monitoring and controlling processes,
//! including CPU/memory analysis, process tree management, and system metrics collection.
//!
//! # Modules
//!
//! - [`parsers`]: Pure parsing functions for /proc filesystem data
//! - [`process_controller`]: Process control operations (suspend, resume, kill)
//! - [`system_interface`]: Trait-based abstraction over OS interactions

pub mod parsers;
pub mod process_controller;
pub mod system_interface;

// Re-export types and functions needed by integration tests and library consumers
pub use system_interface::{MockSystem, RealSystem, SystemInterface, SystemError};
pub use process_controller::ProcessController;

// Re-export from main-specific module (these would normally be in main but are needed for testing)
// Note: These are pulled from the binary's logic and re-exported here for integration testing
pub mod metrics {
    //! Process metrics collection and computation

    // These types and functions will be exposed for library users and integration tests
    // They're currently in main.rs but conceptually belong in a metrics module
    
    /// Represents a single process row in the metrics table
    #[derive(Clone)]
    pub struct ProcRow {
        pub pid: i32,
        pub parent_pid: Option<i32>,
        pub cpu_percent: f32,
        pub mem_percent: f32,
        pub power_watts: Option<f32>,
        pub uptime_secs: u64,
        pub shared_bytes: u64,
        pub virtual_bytes: u64,
        pub resident_bytes: u64,
        pub priority: i64,
        pub name: String,
        pub tree_prefix: String,
        pub tree_marker: String,
    }

    /// Memory information from /proc/meminfo
    pub struct MemInfo {
        pub total: u64,
        pub free: u64,
        pub buffers: u64,
        pub cached: u64,
        pub sreclaimable: u64,
        pub shmem: u64,
        pub available: u64,
    }

    /// Process sorting keys
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum SortKey {
        Cpu,
        Mem,
        Power,
    }
}

// Re-export key metrics types for convenience
pub use metrics::{MemInfo, ProcRow, SortKey};
