//! Simulated SECS/GEM equipment and host.
//!
//! The pair runs on real HSMS-SS sessions over TCP (SEMI E37) carrying
//! SECS-II messages (E5): the host connects, both sides exchange S1F1, the
//! equipment reports S6F11 collection events and S5F1 alarms, and recipes go
//! down with S7F3 and are read back with S7F5/S7F6 to verify their checksum.
//!
//! The implementation rides on [`secs4rs`], a Rust SECS/GEM stack with
//! `#![forbid(unsafe_code)]`. Nothing here is a mock of an internal event bus:
//! the bytes cross a socket.

#[cfg(feature = "secs-gem")]
pub mod pair;

#[cfg(feature = "secs-gem")]
pub use pair::{start_pair, SecsConfig, SecsHandle, SecsStatus, TrafficEntry};

/// The GEM collection events this simulated equipment reports.
pub mod ceid {
    pub const PROCESS_STARTED: i64 = 1001;
    pub const PROCESS_COMPLETED: i64 = 1002;
    pub const EQUIPMENT_UP: i64 = 1003;
}

/// The alarm id used when a chamber fault is injected.
pub const ALID_CHAMBER_FAULT: u32 = 2001;
