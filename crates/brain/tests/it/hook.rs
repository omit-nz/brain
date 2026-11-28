//! `brain hook <point>` as Claude Code runs it: the payload on stdin, never a non-zero exit, and
//! nothing said once it has run over its budget.

use std::fs;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use brain_reflex::{Point, REGISTRY, TRACES, decide};
use omit_agent_trace::{Run, Store};

use super::Home;

fn hook(h: &Home, args: &[&str], payload: &str) -> Output {
    h.brain()
        .arg("hook")
        .args(args)
        .write_stdin(payload.to_string())
        .output()
        .unwrap()
}

fn quiet(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

fn traces(h: &Home) -> Store {
    Store::open(&h.home.path().join("data").join(TRACES)).unwrap()
}

/// A run is dropped by whatever next opens the trace file, daemon or not.
#[test]
fn a_hook_drops_runs_past_their_age() {
    let h = Home::new();
    h.init();
    let said = || {
        quiet(&hook(
            &h,
            &["prompt"],
            r#"{"session_id":"s-1","prompt":"hi"}"#,
        ))
    };
    said();
    let db = rusqlite::Connection::open(h.home.path().join("data/traces.db")).unwrap();
    db.execute("UPDATE runs SET started_ns = 1", []).unwrap();
    said();
    let aged: i64 = db
        .query_row("SELECT count(*) FROM runs WHERE started_ns = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(aged, 0);
}

#[test]
fn a_hook_records_a_run_that_replays() {
    let h = Home::new();
    h.init();
    quiet(&hook(
        &h,
        &["prompt"],
        r#"{"session_id":"s-1","prompt":"hello"}"#,
    ));
    let store = traces(&h);
    let runs = store.runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].name, "hook prompt");
    assert!(runs[0].ended_ns.is_some());
    let env = brain_reflex::Env {
        data: h.home.path().join("data"),
        vault: h.vault.path().into(),
        me: "robin".into(),
        clock: std::sync::Arc::new(brain_platform::clock::SystemClock),
        indexes: Vec::new(),
        trace_wait: brain_reflex::TRACE_WAIT,
    };
    let mut replay = Run::replay(&store, &runs[0].id).unwrap();
    let decided = decide(Point::Prompt, &[], REGISTRY, &env, &mut replay, || {
        panic!("went live")
    })
    .unwrap();
    assert!(decided.failed.is_empty());
    replay.finish().unwrap();
}

#[test]
fn a_hook_never_exits_non_zero() {
    let h = Home::new();
    // No config yet.
    quiet(&hook(&h, &["prompt"], "{}"));
    h.init();
    quiet(&hook(&h, &[], "{}"));
    quiet(&hook(&h, &["nonsense"], "{}"));
    quiet(&hook(&h, &["--help"], "{}"));
    quiet(&hook(&h, &["prompt", "--more", "words"], "{}"));
    quiet(&hook(&h, &["prompt"], "not json"));
    quiet(&hook(&h, &["prompt"], ""));
    fs::write(h.home.path().join("config/config.toml"), "[[[").unwrap();
    quiet(&hook(&h, &["prompt"], "{}"));
}

/// A payload that never arrives: the hook gives up rather than hold the session.
#[test]
fn a_hook_over_budget_says_nothing() {
    let h = Home::new();
    h.init();
    let start = Instant::now();
    let mut child = crate::confined(&crate::built(), h.user.path())
        .args(["hook", "prompt"])
        .env("BRAIN_HOME", h.home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open, so the read of the payload never ends.
    let stdin = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    let took = start.elapsed();
    drop(stdin);
    quiet(&out);
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("250 ms"),
        "{out:?}"
    );
}

#[test]
fn a_hook_error_line_holds_no_payload_text() {
    let h = Home::new();
    h.init();
    let out = hook(&h, &["prompt"], r#"{"prompt":"zeppelin-sentinel""#);
    quiet(&out);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.starts_with("brain: hook prompt:"), "{said}");
    assert!(!said.contains("zeppelin-sentinel"), "{said}");
}
