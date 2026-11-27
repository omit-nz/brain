//! What a reflex may read, and the one way it reads it.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use brain_platform::clock::Clock;
use omit_agent_trace::{Begun, Call, Kind, Reply, Run};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub struct Env {
    /// brain's data folder.
    pub data: PathBuf,
    pub vault: PathBuf,
    pub me: String,
    pub clock: Arc<dyn Clock>,
}

pub struct Cx<'a> {
    env: &'a Env,
    run: &'a mut Run,
    hook: &'a Value,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(env: &'a Env, run: &'a mut Run, hook: &'a Value) -> Cx<'a> {
        Cx { env, run, hook }
    }

    /// The hook payload as Claude Code sent it.
    pub fn hook(&self) -> &'a Value {
        self.hook
    }

    pub(crate) fn run(&mut self) -> &mut Run {
        self.run
    }

    /// Reads something from outside. `live` returns it with the sources whose text it holds
    /// (`note:<id>`, `session:<id>`, `file:<path>`); in a replay it never runs and the
    /// recording answers.
    pub fn read<T: Serialize + DeserializeOwned>(
        &mut self,
        name: &str,
        request: &Value,
        live: impl FnOnce(&Env) -> anyhow::Result<(T, Vec<String>)>,
    ) -> anyhow::Result<T> {
        read(self.env, self.run, name, request, live)
    }
}

pub(crate) fn read<T: Serialize + DeserializeOwned>(
    env: &Env,
    run: &mut Run,
    name: &str,
    request: &Value,
    live: impl FnOnce(&Env) -> anyhow::Result<(T, Vec<String>)>,
) -> anyhow::Result<T> {
    match run.begin(Call::new(Kind::Read, name, request)) {
        Begun::Live(pending) => {
            let read = live(env).and_then(|(value, sources)| {
                let mut reply = Reply::new(serde_json::to_value(&value)?);
                reply.sources = sources;
                Ok((value, reply))
            });
            match read {
                Ok((value, reply)) => {
                    // What `end` hands back is the response just given, already in hand.
                    let _ = run.end(pending, Ok(reply));
                    Ok(value)
                }
                Err(e) => {
                    let _ = run.end(pending, Err(format!("{e:#}")));
                    Err(e)
                }
            }
        }
        Begun::Answered(answer) => Ok(serde_json::from_value(answer.map_err(|e| anyhow!(e))?)?),
    }
}
