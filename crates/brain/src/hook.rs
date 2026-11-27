//! `brain hook <point>`: the one call Claude Code's hooks make. It never fails a session: it
//! exits 0 whatever happens (Claude Code blocks a prompt on exit 2, which is also what clap
//! exits on a word it does not know, so clap never sees these), and it says nothing once it has
//! run over its budget.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::Context;
use brain_platform::clock::SystemClock;
use brain_platform::dirs::Dirs;
use brain_reflex::{Env, Point, REGISTRY, TRACE_WAIT, fire};

use crate::config::Config;
use crate::out::terminal;

/// What a hook may add to a prompt.
pub const BUDGET: Duration = Duration::from_millis(250);

/// The budget a test asked for instead, in a debug build only, so a slow runner is not a failed
/// test.
fn widened() -> Option<Duration> {
    let asked = cfg!(debug_assertions)
        .then(|| std::env::var("BRAIN_TEST_HOOK_BUDGET_MS").ok())
        .flatten();
    asked
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
}

pub fn main(word: Option<String>) -> ExitCode {
    let Some(point) = word.as_deref().and_then(Point::parse) else {
        return ExitCode::SUCCESS;
    };
    // One line, never the panic's own message: it could quote the prompt.
    std::panic::set_hook(Box::new(|_| eprintln!("brain: the hook panicked")));
    let (done, waiting) = mpsc::channel();
    std::thread::spawn(move || done.send(run(point)));
    let name = point.as_str();
    match waiting.recv_timeout(widened().unwrap_or(BUDGET)) {
        Ok(Ok(Some(out))) => {
            // Claude Code may have stopped reading; that is not an error of the session's.
            let _ = writeln!(std::io::stdout(), "{out}");
        }
        Ok(Ok(None)) | Err(RecvTimeoutError::Disconnected) => {}
        Ok(Err(e)) => eprintln!("{}", terminal(&format!("brain: hook {name}: {e:#}"))),
        Err(RecvTimeoutError::Timeout) => eprintln!(
            "brain: hook {name} ran over its {} ms and said nothing",
            BUDGET.as_millis()
        ),
    }
    ExitCode::SUCCESS
}

fn run(point: Point) -> anyhow::Result<Option<String>> {
    let dirs = Dirs::resolve()?;
    let config = Config::load(&dirs)?;
    let mut payload = String::new();
    std::io::stdin().read_to_string(&mut payload)?;
    let payload = serde_json::from_str(&payload).context("the payload is not JSON")?;
    let env = Env {
        data: dirs.data,
        vault: config.vault,
        me: config.me,
        clock: Arc::new(SystemClock),
        trace_wait: widened().unwrap_or(TRACE_WAIT),
    };
    fire(point, payload, REGISTRY, &env)
}
