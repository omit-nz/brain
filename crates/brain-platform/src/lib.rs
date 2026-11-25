//! The one place brain touches the operating system: folders, file writes and moves, code signatures, and the clock.

pub mod clock;
pub mod codesign;
pub mod dirs;
pub mod fs;
pub mod process;
pub mod service;
