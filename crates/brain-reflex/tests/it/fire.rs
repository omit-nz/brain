use brain_reflex::{Point, REGISTRY, Reflex, Stage, TRACES, VERDICTS, fire};
use omit_agent_trace::Kind;
use serde_json::{Value, json};

use crate::{Home, LIVE, SHADOW};

fn said(out: Option<String>) -> Option<String> {
    out.map(|o| {
        let v: Value = serde_json::from_str(&o).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_string()
    })
}

#[test]
fn a_fire_is_traced_step_by_step() {
    let home = Home::temp();
    let payload = home.payload("s-1", "alpha\nbravo\n");
    let file = payload["file"].as_str().unwrap().to_string();
    fire(Point::Prompt, payload, &[SHADOW], &home.env()).unwrap();

    let runs = home.runs();
    assert_eq!(runs.len(), 1);
    let (_, name, spans) = &runs[0];
    assert_eq!(name, "hook prompt");
    let shape: Vec<_> = spans.iter().map(|s| (s.kind, s.name.as_str())).collect();
    assert_eq!(
        shape,
        [
            (Kind::Read, "hook"),
            (Kind::Step, "fixture"),
            (Kind::Read, "lines"),
            (Kind::Read, "passed-keys"),
        ]
    );
    assert_eq!(spans[0].sources, ["session:s-1"]);
    assert_eq!(spans[2].sources, [format!("file:{file}")]);
    assert_eq!(spans[0].parent, None);
    assert_eq!(spans[2].parent.as_ref(), Some(&spans[1].id));
    assert_eq!(spans[3].parent.as_ref(), Some(&spans[1].id));
    let (event, attributes) = &spans[1].events[0];
    assert_eq!(event, "verdict");
    assert_eq!(attributes["reflex"], "fixture");
    assert_eq!(attributes["version"], 1);
    assert_eq!(attributes["fired"], true);
    assert_eq!(attributes["acted"], false);
    assert!(spans.iter().all(|s| s.ended_ns.is_some()));
}

#[test]
fn a_shadow_reflex_logs_and_says_nothing() {
    let home = Home::temp();
    let out = fire(
        Point::Prompt,
        home.payload("s-1", "alpha\n"),
        &[SHADOW],
        &home.env(),
    )
    .unwrap();
    assert_eq!(out, None);
    let rows = home.verdicts().rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reflex, "fixture");
    assert_eq!(rows[0].session.as_deref(), Some("s-1"));
    assert_eq!(rows[0].input_text, "what now?");
    assert!(rows[0].fired);
    assert!(!rows[0].acted);
    // Logged as if live, so the log shows what going live would show.
    assert_eq!(
        rows[0].picks.iter().map(|p| p.passed).collect::<Vec<_>>(),
        [true]
    );
    assert_eq!(rows[0].run, Some(home.runs()[0].0.clone()));
}

#[test]
fn a_live_reflex_says_its_picks_once_a_session() {
    let home = Home::temp();
    let env = home.env();
    let first = fire(Point::Prompt, home.payload("s-1", "alpha\n"), &[LIVE], &env).unwrap();
    assert_eq!(said(first).as_deref(), Some("alpha"));
    let second = fire(
        Point::Prompt,
        home.payload("s-1", "alpha\nbravo\n"),
        &[LIVE],
        &env,
    )
    .unwrap();
    assert_eq!(said(second).as_deref(), Some("bravo"));
    let third = fire(
        Point::Prompt,
        home.payload("s-1", "alpha\nbravo\n"),
        &[LIVE],
        &env,
    )
    .unwrap();
    assert_eq!(third, None);
    let other = fire(Point::Prompt, home.payload("s-2", "alpha\n"), &[LIVE], &env).unwrap();
    assert_eq!(said(other).as_deref(), Some("alpha"));
}

#[test]
fn a_reflex_under_its_floor_logs_and_says_nothing() {
    let home = Home::temp();
    let out = fire(
        Point::Prompt,
        home.payload("s-1", "ab\n"),
        &[LIVE],
        &home.env(),
    )
    .unwrap();
    assert_eq!(out, None);
    let rows = home.verdicts().rows().unwrap();
    assert!(rows[0].fired && !rows[0].acted);
    assert!(!rows[0].picks[0].passed);
}

/// The row is there before the words are: a verdict that cannot be written is not said.
#[test]
fn a_verdict_is_written_before_it_is_said() {
    let home = Home::temp();
    let env = home.env();
    std::fs::create_dir_all(&env.data).unwrap();
    home.verdicts();
    rusqlite::Connection::open(env.data.join(VERDICTS))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no BEFORE INSERT ON verdicts BEGIN SELECT RAISE(ABORT, 'no'); END",
        )
        .unwrap();
    let out = fire(Point::Prompt, home.payload("s-1", "alpha\n"), &[LIVE], &env);
    assert!(out.is_err(), "{out:?}");
}

#[test]
fn one_reflex_failing_leaves_the_others_their_say() {
    fn broken(_: &mut brain_reflex::Cx<'_>) -> anyhow::Result<Option<brain_reflex::Input>> {
        anyhow::bail!("broken")
    }
    let broken = Reflex {
        name: "broken",
        extract: broken,
        ..LIVE
    };
    let home = Home::temp();
    let out = fire(
        Point::Prompt,
        home.payload("s-1", "alpha\n"),
        &[broken, LIVE],
        &home.env(),
    )
    .unwrap();
    assert_eq!(said(out).as_deref(), Some("alpha"));
}

#[test]
fn an_event_for_no_reflex_is_still_recorded() {
    let home = Home::temp();
    let out = fire(
        Point::Stop,
        json!({ "session_id": "s-1" }),
        &[SHADOW],
        &home.env(),
    )
    .unwrap();
    assert_eq!(out, None);
    let runs = home.runs();
    assert_eq!(runs[0].1, "hook stop");
    assert_eq!(runs[0].2.len(), 1);
}

#[test]
fn a_trace_store_that_cannot_open_never_changes_the_hook() {
    let home = Home::temp();
    let env = home.env();
    std::fs::create_dir_all(env.data.join(TRACES)).unwrap();
    let out = fire(Point::Prompt, home.payload("s-1", "alpha\n"), &[LIVE], &env).unwrap();
    assert_eq!(said(out).as_deref(), Some("alpha"));
    assert_eq!(home.verdicts().rows().unwrap()[0].run, None);
}

#[cfg(unix)]
#[test]
fn the_trace_and_verdict_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::temp();
    let env = home.env();
    fire(
        Point::Prompt,
        home.payload("s-1", "alpha\n"),
        &[SHADOW],
        &env,
    )
    .unwrap();
    for path in [
        env.data.clone(),
        env.data.join(TRACES),
        env.data.join(VERDICTS),
    ] {
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{}", path.display());
    }
}

fn live(registry: &[Reflex]) -> Vec<&str> {
    registry
        .iter()
        .filter(|r| r.stage == Stage::Live)
        .map(|r| r.name)
        .collect()
}

#[test]
fn every_registered_reflex_ships_in_shadow() {
    assert_eq!(live(REGISTRY), Vec::<&str>::new());
    assert_eq!(live(&[SHADOW, LIVE]), ["fixture"]);
}

#[test]
fn old_unlabelled_verdicts_are_pruned() {
    let home = Home::temp();
    let env = home.env();
    for session in ["old", "labelled", "pick-labelled"] {
        fire(
            Point::Prompt,
            home.payload(session, "alpha\n"),
            &[SHADOW],
            &env,
        )
        .unwrap();
    }
    let db = rusqlite::Connection::open(env.data.join(VERDICTS)).unwrap();
    db.execute(
        "UPDATE verdicts SET label = 'right' WHERE session = 'labelled'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE picks SET label = 'wrong' WHERE verdict_id IN
             (SELECT id FROM verdicts WHERE session = 'pick-labelled')",
        [],
    )
    .unwrap();
    let verdicts = home.verdicts();
    let day = |s: &str| s.parse::<jiff::Timestamp>().unwrap();
    // The 89th day after: nothing goes.
    assert_eq!(verdicts.prune(day("2027-02-24T10:00:00Z")).unwrap(), 0);
    assert_eq!(verdicts.prune(day("2027-02-26T10:00:00Z")).unwrap(), 1);
    let left: Vec<_> = verdicts
        .rows()
        .unwrap()
        .into_iter()
        .map(|r| r.session.unwrap())
        .collect();
    assert_eq!(left, ["labelled", "pick-labelled"]);
}
