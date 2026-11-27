mod fire;
mod replay;

use std::sync::Arc;

use brain_platform::clock::FixedClock;
use brain_reflex::{
    Act, Backend, Candidate, Cx, Env, Input, Point, Reflex, Stage, TRACES, Verdict, Verdicts,
};
use omit_agent_trace::{Span, Store};
use serde_json::{Value, json};

pub struct Home {
    pub dir: tempfile::TempDir,
}

impl Home {
    pub fn temp() -> Home {
        Home {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    pub fn env(&self) -> Env {
        Env {
            data: self.dir.path().join("data"),
            vault: self.dir.path().join("vault"),
            me: "robin".into(),
            clock: Arc::new(FixedClock("2026-11-27T10:00:00Z".parse().unwrap())),
        }
    }

    /// A file the fixture reflex reads, and the hook payload that names it.
    pub fn payload(&self, session: &str, lines: &str) -> Value {
        let file = self.dir.path().join("lines.txt");
        std::fs::write(&file, lines).unwrap();
        json!({ "session_id": session, "prompt": "what now?", "file": file.to_str().unwrap() })
    }

    pub fn verdicts(&self) -> Verdicts {
        Verdicts::open(&self.env().data.join(brain_reflex::VERDICTS)).unwrap()
    }

    /// The one run recorded, by name and spans.
    pub fn runs(&self) -> Vec<(String, String, Vec<Span>)> {
        let store = Store::open(&self.env().data.join(TRACES)).unwrap();
        store
            .runs()
            .unwrap()
            .into_iter()
            .map(|r| {
                let spans = store.spans(&r.id).unwrap();
                (r.id, r.name, spans)
            })
            .collect()
    }
}

/// Offers one candidate per line of the file the payload names.
fn lines(cx: &mut Cx<'_>) -> anyhow::Result<Option<Input>> {
    let Some(path) = cx.hook()["file"].as_str().map(str::to_string) else {
        return Ok(None);
    };
    let text: String = cx.read("lines", &json!({ "file": path }), |_| {
        Ok((
            std::fs::read_to_string(&path)?,
            vec![format!("file:{path}")],
        ))
    })?;
    let candidates = text
        .lines()
        .map(|l| Candidate {
            key: l.into(),
            score: l.len() as f32 / 7.0,
            text: l.into(),
        })
        .collect();
    Ok(Some(Input {
        text: cx.hook()["prompt"].as_str().unwrap_or_default().into(),
        candidates,
    }))
}

fn best(input: &Input) -> Verdict {
    Verdict {
        fired: !input.candidates.is_empty(),
        confidence: input.candidates.iter().map(|c| c.score).fold(0.0, f32::max),
        reason: "lines".into(),
        picks: input.candidates.clone(),
    }
}

fn say(_: &Input, v: &Verdict) -> String {
    v.picks
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

pub const SHADOW: Reflex = Reflex {
    name: "fixture",
    version: 1,
    point: Point::Prompt,
    extract: lines,
    rule: best,
    backend: Backend::Rule,
    floor: 0.5,
    act: Act::Inject(say),
    stage: Stage::Shadow,
};

pub const LIVE: Reflex = Reflex {
    stage: Stage::Live,
    ..SHADOW
};
