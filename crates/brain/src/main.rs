//! `brain`: exit 0 when it worked, 1 on an error, 2 on a usage mistake.

mod capture;
mod cli;
mod config;
mod connect;
mod daemon;
mod gaps;
mod import;
mod inbox;
mod mcp;
mod out;
mod runtime;
mod usage;

use std::process::ExitCode;

use brain_platform::dirs::Dirs;
use clap::Parser;

fn main() -> ExitCode {
    let result = Dirs::resolve().and_then(|dirs| {
        runtime::prepare(&dirs)?;
        cli::run(cli::Cli::parse(), &dirs)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", out::terminal(&format!("brain: {e:#}")));
            ExitCode::from(1)
        }
    }
}
