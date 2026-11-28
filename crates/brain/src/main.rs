//! `brain`: exit 0 when it worked, 1 on an error, 2 on a usage mistake.

mod calls;
mod capture;
mod cli;
mod config;
mod connect;
mod daemon;
mod gaps;
mod hook;
mod import;
mod inbox;
mod mcp;
mod offer;
mod open;
mod plugin;
mod room;
mod runtime;
mod usage;

use std::process::ExitCode;

use brain::{brief, out};

use brain_platform::dirs::Dirs;
use clap::Parser;

fn main() -> ExitCode {
    // A hook is taken before anything that could exit non-zero or load a model.
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a == "hook") {
        return hook::main(args.next().and_then(|a| a.into_string().ok()));
    }
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
