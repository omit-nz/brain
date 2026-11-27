//! A hook point: run its reflexes, log every verdict, say what the live ones say.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use brain_platform::fs::create_dir_private;
use omit_agent_trace::{Kind, Run, Store};
use serde_json::{Value, json};

use crate::cx::{self, Cx, Env};
use crate::store::{PickRow, Row, Verdicts};
use crate::{Act, Input, Point, Reflex, Stage, TRACES, VERDICTS, Verdict};

/// How long a hook waits for the trace store before it runs untraced. Opening it is a write, so
/// another writer would be waited on for five seconds.
const TRACE_WAIT: Duration = Duration::from_millis(150);

/// The backend that decides at the hook.
const RULE: &str = "rule";

/// What one reflex made of the hook.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub reflex: String,
    pub version: u32,
    pub input: Input,
    /// The rule's verdict, with everything it picked.
    pub verdict: Verdict,
    /// Per pick: whether it would be said, which a key is once a session.
    pub passed: Vec<bool>,
    pub say: Option<String>,
    pub latency_ms: i64,
    /// Whose text the decision rests on. A replay reads nothing, so it names none.
    pub sources: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct Decided {
    pub decisions: Vec<Decision>,
    /// The reflexes that failed, by name; the others still had their say.
    pub failed: Vec<&'static str>,
}

/// The session a payload names.
fn session(hook: &Value) -> Option<&str> {
    hook["session_id"].as_str().filter(|s| !s.is_empty())
}

/// Runs the point's reflexes over the payload, reading only through `run`, and writes nothing
/// but the trace. Given a replaying run it reads and does nothing for real.
pub fn decide(
    point: Point,
    registry: &[Reflex],
    env: &Env,
    run: &mut Run,
    payload: impl FnOnce() -> anyhow::Result<Value>,
) -> anyhow::Result<Decided> {
    let mut payload_sources = BTreeSet::new();
    let hook: Value = cx::read(env, run, &mut payload_sources, "hook", &Value::Null, |_| {
        let hook = payload()?;
        let sources = session(&hook)
            .map(|s| format!("session:{s}"))
            .into_iter()
            .collect();
        Ok((hook, sources))
    })?;
    let mut out = Decided::default();
    for reflex in registry.iter().filter(|r| r.point == point) {
        let mut sources = payload_sources.clone();
        let decided = run.span(Kind::Step, reflex.name, |run| {
            one(reflex, &mut Cx::new(env, run, &hook, &mut sources)).map_err(|e| format!("{e:#}"))
        });
        let decided = decided.map(|d| d.map(|d| Decision { sources, ..d }));
        match decided {
            Ok(decision) => out.decisions.extend(decision),
            Err(_) => out.failed.push(reflex.name),
        }
    }
    Ok(out)
}

fn one(reflex: &Reflex, cx: &mut Cx<'_>) -> anyhow::Result<Option<Decision>> {
    let start = Instant::now();
    let Some(input) = (reflex.extract)(cx)? else {
        return Ok(None);
    };
    let verdict = (reflex.rule)(&input);
    // What a live reflex would say. A shadow one is logged as if live, so its log shows what
    // going live would show.
    let clears = verdict.fired && verdict.confidence >= reflex.floor;
    let mut seen: HashSet<String> = match session(cx.hook()) {
        Some(session) if clears && !verdict.picks.is_empty() => {
            let name = reflex.name;
            let request = json!({ "reflex": name, "session": session });
            cx.read("passed-keys", &request, |env| {
                let keys =
                    Verdicts::open(&env.data.join(VERDICTS))?.passed_keys(name, RULE, session)?;
                Ok((keys, Vec::new()))
            })?
        }
        _ => HashSet::new(),
    };
    let passed: Vec<bool> = verdict
        .picks
        .iter()
        .map(|p| clears && seen.insert(p.key.clone()))
        .collect();
    // A verdict whose picks have all been said has nothing left to say.
    let speaks = clears && (verdict.picks.is_empty() || passed.contains(&true));
    let say = match reflex.act {
        Act::Inject(say) if reflex.stage == Stage::Live && speaks => {
            let picks = verdict
                .picks
                .iter()
                .zip(&passed)
                .filter(|(_, passed)| **passed)
                .map(|(pick, _)| pick.clone())
                .collect();
            Some(say(
                &input,
                &Verdict {
                    picks,
                    ..verdict.clone()
                },
            ))
        }
        _ => None,
    };
    cx.run().event(
        "verdict",
        &json!({
            "reflex": reflex.name,
            "version": reflex.version,
            "fired": verdict.fired,
            "confidence": verdict.confidence,
            "reason": verdict.reason,
            "acted": say.is_some(),
        }),
    );
    Ok(Some(Decision {
        reflex: reflex.name.into(),
        version: reflex.version,
        input,
        verdict,
        passed,
        say,
        latency_ms: start.elapsed().as_millis() as i64,
        sources: BTreeSet::new(),
    }))
}

/// Records a run, decides, logs every verdict and returns the hook's JSON, or `None` when there
/// is nothing to say. A verdict is written before it is returned to be said.
pub fn fire(
    point: Point,
    payload: Value,
    registry: &[Reflex],
    env: &Env,
) -> anyhow::Result<Option<String>> {
    create_dir_private(&env.data)?;
    let mut run = record(env.data.join(TRACES), format!("hook {}", point.as_str()));
    let traced = run.id().map(str::to_string);
    let session = session(&payload).map(str::to_string);
    let decided = decide(point, registry, env, &mut run, || Ok(payload))?;
    for name in &decided.failed {
        eprintln!("brain: the {name} reflex failed");
    }
    if let Err(e) = run.finish() {
        eprintln!("brain: this hook was not fully traced: {e}");
    }
    // The last write before the words go out, so a row never says what a hook that ran out of
    // time did not say.
    log(point, env, session, traced, &decided).map_err(|e| anyhow!("writing a verdict: {e:#}"))?;
    let said: Vec<String> = decided
        .decisions
        .into_iter()
        .filter_map(|d| d.say)
        .collect();
    Ok((!said.is_empty()).then(|| {
        json!({
            "hookSpecificOutput": {
                "hookEventName": point.event_name(),
                "additionalContext": said.join("\n\n"),
            }
        })
        .to_string()
    }))
}

/// A run recorded in the trace store, or one that records nothing when the store cannot be
/// opened in time: a trace that cannot be written never changes what the hook does.
fn record(path: PathBuf, name: String) -> Run {
    let (done, waiting) = mpsc::channel();
    std::thread::spawn(move || done.send(Store::open(&path).map(|s| Run::record(&s, &name))));
    match waiting.recv_timeout(TRACE_WAIT) {
        Ok(Ok(run)) => run,
        Ok(Err(e)) => {
            eprintln!("brain: this hook is not traced: {e}");
            Run::off()
        }
        Err(_) => {
            eprintln!("brain: this hook is not traced: the trace store is busy");
            Run::off()
        }
    }
}

fn log(
    point: Point,
    env: &Env,
    session: Option<String>,
    run: Option<String>,
    decided: &Decided,
) -> anyhow::Result<()> {
    if decided.decisions.is_empty() {
        return Ok(());
    }
    let mut verdicts = Verdicts::open(&env.data.join(VERDICTS))?;
    for d in &decided.decisions {
        verdicts.insert(&Row {
            reflex: d.reflex.clone(),
            version: d.version,
            backend: RULE.into(),
            point: point.as_str().into(),
            sources: d.sources.iter().cloned().collect(),
            at: env.clock.now(),
            session: session.clone(),
            run: run.clone(),
            input_text: d.input.text.clone(),
            fired: d.verdict.fired,
            confidence: d.verdict.confidence,
            reason: d.verdict.reason.clone(),
            acted: d.say.is_some(),
            latency_ms: d.latency_ms,
            picks: d
                .verdict
                .picks
                .iter()
                .zip(&d.passed)
                .map(|(p, passed)| PickRow {
                    key: p.key.clone(),
                    score: p.score,
                    text: p.text.clone(),
                    passed: *passed,
                })
                .collect(),
        })?;
    }
    Ok(())
}
