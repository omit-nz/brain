use brain_reflex::{Cx, Input, Point, Reflex, TRACES, decide, fire};
use omit_agent_trace::{Run, Store};
use serde_json::json;

use crate::{Home, LIVE, SHADOW};

/// Build item 0's done line: a brain test replays a recorded run, with nothing left to read.
#[test]
fn a_recorded_run_replays_with_no_live_call() {
    let home = Home::temp();
    let env = home.env();
    let payload = home.payload("s-1", "alpha\nbravo\n");
    let file = std::path::PathBuf::from(payload["file"].as_str().unwrap());
    fire(Point::Prompt, payload, &[], &[LIVE], &env).unwrap();
    let rows = home.verdicts().rows().unwrap();
    let id = home.runs()[0].0.clone();
    std::fs::remove_file(&file).unwrap();
    std::fs::remove_file(env.data.join(brain_reflex::VERDICTS)).unwrap();

    let store = Store::open(&env.data.join(TRACES)).unwrap();
    let mut run = Run::replay(&store, &id).unwrap();
    let again = decide(Point::Prompt, &[], &[LIVE], &env, &mut run, || {
        panic!("went live")
    })
    .unwrap();
    run.finish().unwrap();

    assert!(again.failed.is_empty());
    let d = &again.decisions[0];
    assert_eq!(d.reflex, rows[0].reflex);
    assert_eq!(d.verdict.fired, rows[0].fired);
    assert_eq!(d.verdict.confidence.to_bits(), rows[0].confidence.to_bits());
    let picks: Vec<_> = d
        .verdict
        .picks
        .iter()
        .zip(&d.passed)
        .map(|(p, passed)| (p.key.as_str(), p.score.to_bits(), *passed))
        .collect();
    let logged: Vec<_> = rows[0]
        .picks
        .iter()
        .map(|p| (p.key.as_str(), p.score.to_bits(), p.passed))
        .collect();
    assert_eq!(picks, logged);
    assert_eq!(d.say.as_deref(), Some("alpha,bravo"));
    assert!(
        !env.data.join(brain_reflex::VERDICTS).exists(),
        "a replay writes nothing"
    );
}

/// A reflex that now reads something the recording does not hold is a miss, never a live read.
#[test]
fn a_changed_reflex_fails_the_replay() {
    fn reads_more(cx: &mut Cx<'_>) -> anyhow::Result<Option<Input>> {
        cx.read::<String>("another", &json!(null), |_| panic!("went live"))?;
        Ok(None)
    }
    let changed = Reflex {
        extract: reads_more,
        ..SHADOW
    };
    let home = Home::temp();
    let env = home.env();
    fire(
        Point::Prompt,
        home.payload("s-1", "alpha\n"),
        &[],
        &[SHADOW],
        &env,
    )
    .unwrap();
    let store = Store::open(&env.data.join(TRACES)).unwrap();
    let mut run = Run::replay(&store, &home.runs()[0].0).unwrap();
    let again = decide(Point::Prompt, &[], &[changed], &env, &mut run, || {
        panic!("went live")
    })
    .unwrap();
    assert_eq!(again.failed, ["fixture"]);
    assert!(run.finish().is_err());
}

/// What is scrubbed from a trace is scrubbed from what a reflex sees, so both see the same.
#[test]
fn a_replay_sees_what_the_live_run_saw() {
    fn asks_with_what_it_read(cx: &mut Cx<'_>) -> anyhow::Result<Option<Input>> {
        let prompt = cx.hook()["prompt"].as_str().unwrap_or_default().to_string();
        let found: String = cx.read("search", &json!({ "words": prompt }), |_| {
            // Assembled here, so no scanner takes the fixture for a real key.
            Ok((
                format!("the token={}{} line", "abc123", "def456"),
                Vec::new(),
            ))
        })?;
        let again: String = cx.read("follow", &json!({ "found": found }), |_| {
            Ok((found.clone(), Vec::new()))
        })?;
        Ok(Some(Input {
            text: format!("{prompt} / {again}"),
            candidates: Vec::new(),
        }))
    }
    let reflex = Reflex {
        extract: asks_with_what_it_read,
        ..SHADOW
    };
    let home = Home::temp();
    let env = home.env();
    let payload = json!({ "session_id": "s-1", "prompt": "the wifi password: hunter2 changed" });
    fire(Point::Prompt, payload, &[], &[reflex], &env).unwrap();
    let rows = home.verdicts().rows().unwrap();
    assert!(
        !rows[0].input_text.contains("hunter2"),
        "{}",
        rows[0].input_text
    );
    assert!(
        !rows[0].input_text.contains("def456"),
        "{}",
        rows[0].input_text
    );

    let store = Store::open(&env.data.join(TRACES)).unwrap();
    let mut run = Run::replay(&store, &home.runs()[0].0).unwrap();
    let again = decide(Point::Prompt, &[], &[reflex], &env, &mut run, || {
        panic!("went live")
    })
    .unwrap();
    assert!(again.failed.is_empty(), "{:?}", again.failed);
    assert_eq!(again.decisions[0].input.text, rows[0].input_text);
    run.finish().unwrap();
}
