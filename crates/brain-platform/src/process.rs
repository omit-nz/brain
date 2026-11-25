//! Handing this process over to another program.

use std::ffi::OsString;
use std::io;
use std::path::Path;

/// Runs `program` with `args` in place of this process, so whoever started this one (an AI app
/// speaking MCP over stdin and stdout) talks to `program` directly. On Unix the process image is
/// replaced; elsewhere `program` runs as a child and this process exits with its code. Returns
/// only when `program` could not be started.
pub fn replace(program: &Path, args: &[OsString]) -> io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(program).args(args).exec()
    }
    #[cfg(not(unix))]
    {
        match std::process::Command::new(program).args(args).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(e) => e,
        }
    }
}
