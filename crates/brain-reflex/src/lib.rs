//! Reflexes: small deciders that run at Claude Code's hook points and decide what, if anything,
//! to put in front of the AI. Everything a reflex reads from outside goes through its `Cx`, so
//! every run is traced and a recorded one replays with nothing read again.

mod cx;
mod fire;
mod store;

pub use cx::{Cx, Env};
pub use fire::{Decided, Decision, TRACE_WAIT, decide, fire};
pub use store::{PickRow, Row, Verdicts};

use std::path::Path;

use jiff::Timestamp;
use omit_agent_trace::Store;
use serde::{Deserialize, Serialize};

/// The trace store and the verdict store, in brain's data folder.
pub const TRACES: &str = "traces.db";
pub const VERDICTS: &str = "verdicts.db";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    SessionStart,
    Prompt,
    Stop,
    SessionEnd,
}

impl Point {
    const ALL: [Point; 4] = [
        Point::SessionStart,
        Point::Prompt,
        Point::Stop,
        Point::SessionEnd,
    ];

    /// The word `brain hook` takes.
    pub fn as_str(self) -> &'static str {
        match self {
            Point::SessionStart => "session-start",
            Point::Prompt => "prompt",
            Point::Stop => "stop",
            Point::SessionEnd => "session-end",
        }
    }

    /// The hook's name as Claude Code spells it.
    pub fn event_name(self) -> &'static str {
        match self {
            Point::SessionStart => "SessionStart",
            Point::Prompt => "UserPromptSubmit",
            Point::Stop => "Stop",
            Point::SessionEnd => "SessionEnd",
        }
    }

    pub fn parse(word: &str) -> Option<Point> {
        Point::ALL.into_iter().find(|p| p.as_str() == word)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Shadow,
    Live,
}

/// `Rule` decides at the hook; `Embed` scores logged verdicts afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Rule,
    Embed,
}

#[derive(Clone, Copy)]
pub enum Act {
    Inject(fn(&Input, &Verdict) -> String),
    Log,
}

/// Something a reflex may choose to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Names it across verdicts: a session is shown a key once.
    pub key: String,
    pub score: f32,
    /// What would be shown for it.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub text: String,
    /// What there is to choose among. Empty for a reflex that only classifies.
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub fired: bool,
    pub confidence: f32,
    pub reason: String,
    /// What it chose, best first.
    pub picks: Vec<Candidate>,
}

#[derive(Clone, Copy)]
pub struct Reflex {
    pub name: &'static str,
    /// Bumped when behaviour changes; labels and eval key on it.
    pub version: u32,
    pub point: Point,
    /// `None`: this event is not for this reflex.
    pub extract: fn(&mut Cx<'_>) -> anyhow::Result<Option<Input>>,
    pub rule: fn(&Input) -> Verdict,
    pub backend: Backend,
    /// Act only at or above this confidence.
    pub floor: f32,
    pub act: Act,
    pub stage: Stage,
}

/// Context a hook point always gets: it judges nothing, so it has no verdict and no shadow. It is
/// traced as a reflex is, and reads the same way.
#[derive(Clone, Copy)]
pub struct Standing {
    pub name: &'static str,
    pub point: Point,
    pub say: fn(&mut Cx<'_>) -> anyhow::Result<Option<String>>,
}

/// The reflexes brain ships. Every one ships in shadow.
pub const REGISTRY: &[Reflex] = &[];

/// Drops what is past keeping in `data`: traces after 30 days, unlabelled verdicts after 90.
/// Where brain has recorded nothing, nothing is made.
pub fn prune(data: &Path, now: Timestamp) -> anyhow::Result<()> {
    let traces = data.join(TRACES);
    if traces.exists() {
        // Opening the trace store prunes it.
        Store::open(&traces)?;
    }
    let verdicts = data.join(VERDICTS);
    if verdicts.exists() {
        Verdicts::open(&verdicts)?.prune(now)?;
    }
    Ok(())
}

/// Removes every trace and verdict that holds one of `sources`' text.
pub fn purge(data: &Path, sources: &[String]) -> anyhow::Result<()> {
    if sources.is_empty() {
        return Ok(());
    }
    let verdicts = data.join(VERDICTS);
    if verdicts.exists() {
        let verdicts = Verdicts::open(&verdicts)?;
        for source in sources {
            verdicts.purge(source)?;
        }
    }
    let traces = data.join(TRACES);
    if traces.exists() {
        let traces = Store::open(&traces)?;
        for source in sources {
            traces.purge(source)?;
        }
    }
    Ok(())
}
