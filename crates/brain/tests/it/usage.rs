//! What brain counts about its own use: kinds and doors and numbers, never text, and never at the
//! cost of the action counted.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use brain_report::{Door, Event, accept};
use serde_json::{Value, json};

use super::Home;
use super::mcp::{Mcp, indexed};

const SENTINEL: &str = "ZQX-SENTINEL-7";

/// `brain usage`'s count lines, `kind  door  n`.
fn counts(h: &Home) -> Vec<String> {
    h.run(&["usage"])
        .lines()
        .skip_while(|l| *l != "Today so far:")
        .skip(1)
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn usage_sending_is_on_by_default() {
    let h = Home::new();
    h.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        h.vault.path().to_str().unwrap(),
    ]);
    let config = fs::read_to_string(h.home.path().join("config/config.toml")).unwrap();
    assert!(!config.contains("[usage]"), "{config}");
    let out = h.run(&["usage"]);
    assert!(
        out.starts_with("Usage sending: on, to https://usage.brain.omit.nz/v1/day"),
        "{out}"
    );
}

#[test]
fn usage_off_is_saved() {
    let h = Home::new();
    h.init();
    h.run(&["usage", "off"]);
    assert!(h.run(&["usage"]).starts_with("Usage sending: off"));
    h.run(&["usage", "on"]);
    assert!(h.run(&["usage"]).starts_with("Usage sending: on"));
}

#[test]
fn a_second_init_keeps_usage_off() {
    let h = Home::new();
    h.init();
    h.run(&["usage", "off"]);
    h.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        h.vault.path().to_str().unwrap(),
    ]);
    assert!(h.run(&["usage"]).starts_with("Usage sending: off"));
}

#[test]
fn counting_continues_while_sending_is_off() {
    let h = indexed(&[("a.md", "mailbox notes\n")]);
    h.run(&["usage", "off"]);
    h.run(&["search", "mailbox"]);
    assert!(
        counts(&h).contains(&"question cli 1".to_string()),
        "off keeps the counts on this machine; it stops only the sending"
    );
}

#[test]
fn the_mcp_door_counts_captures_corrections_and_gaps() {
    let h = indexed(&[("bins.md", "---\ntitle: Bins\n---\nMondays\n")]);
    let mut m = Mcp::start(&h);
    m.text("capture", json!({"text": "invoices on the 20th"}));
    m.text(
        "correct",
        json!({"target": "bins.md", "correction": "Tuesdays"}),
    );
    m.text("search_my_brain", json!({"query": "zeppelin"}));
    drop(m);
    let c = counts(&h);
    for line in [
        "capture_written claude_code 1",
        "correction claude_code 1",
        "gap_logged claude_code 1",
    ] {
        assert!(c.contains(&line.to_string()), "{line}: {c:?}");
    }
}

#[test]
fn a_store_that_cannot_be_written_says_so_once_per_action() {
    let h = indexed(&[("a.md", "mailbox\n")]);
    let store = h.home.path().join("data/activity.db");
    let _ = fs::remove_file(&store);
    fs::create_dir_all(&store).unwrap();
    for query in ["mailbox", "zeppelin"] {
        let out = h.brain().args(["search", query]).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(err.matches("was not saved").count(), 1, "{query}: {err}");
    }
}

#[test]
fn a_search_counts_a_question_by_door() {
    let h = indexed(&[("a.md", "mailbox notes\n")]);
    h.run(&["search", "mailbox"]);
    let mut m = Mcp::start(&h);
    m.text("search_my_brain", json!({"query": "mailbox"}));
    drop(m);
    let c = counts(&h);
    assert!(c.contains(&"question cli 1".to_string()), "{c:?}");
    assert!(c.contains(&"question claude_code 1".to_string()), "{c:?}");
    let latency: usize = c.iter().filter(|l| l.starts_with("search_")).count();
    assert!(latency >= 1, "a latency bucket per door: {c:?}");
}

#[test]
fn an_answer_from_a_kept_note_is_counted() {
    let h = indexed(&[
        ("3-permanent/a.md", "mailbox notes\n"),
        ("0-inbox/b.md", "kettle\n"),
    ]);
    h.run(&["search", "mailbox"]);
    h.run(&["search", "kettle"]);
    let c = counts(&h);
    assert!(c.contains(&"answer_kept cli 1".to_string()), "{c:?}");
}

/// A kept note listed only because it supersedes an inbox hit did not answer anything.
#[test]
fn an_added_overtaker_is_never_counted_as_an_answer() {
    let h = indexed(&[
        ("0-inbox/old.md", "mailbox notes\n"),
        ("3-permanent/new.md", "---\nsupersedes: old\n---\nlater\n"),
    ]);
    let shown = h.run(&["search", "mailbox"]);
    assert!(shown.contains("3-permanent/new.md"), "{shown}");
    let c = counts(&h);
    assert!(!c.iter().any(|l| l.starts_with("answer_kept")), "{c:?}");
}

#[test]
fn a_gap_is_never_counted_as_an_answer() {
    let h = indexed(&[("3-permanent/a.md", "mailbox notes\n")]);
    h.run(&["search", "zeppelin"]);
    let c = counts(&h);
    assert!(c.contains(&"gap_logged cli 1".to_string()), "{c:?}");
    assert!(!c.iter().any(|l| l.starts_with("answer_kept")), "{c:?}");
}

#[test]
fn inbox_keep_and_skip_are_counted() {
    let h = Home::new();
    h.init();
    h.run(&["add", "one", "--title", "One"]);
    h.run(&["add", "two", "--title", "Two"]);
    h.run(&["inbox", "keep", "0-inbox/one.md"]);
    h.run(&["inbox", "skip", "0-inbox/two.md"]);
    let c = counts(&h);
    for line in [
        "capture_written cli 2",
        "capture_kept cli 1",
        "capture_skipped cli 1",
    ] {
        assert!(c.contains(&line.to_string()), "{line}: {c:?}");
    }
}

#[test]
fn counts_never_hold_text() {
    let h = indexed(&[(
        &format!("{SENTINEL}/{SENTINEL}.md"),
        &format!("---\ntitle: {SENTINEL}\n---\n{SENTINEL} body\n"),
    )]);
    h.run(&["search", SENTINEL]);
    h.run(&["search", &format!("{SENTINEL} nowhere zeppelin")]);
    h.run(&["add", SENTINEL, "--title", SENTINEL]);
    h.run(&["correct", &format!("{SENTINEL}/{SENTINEL}.md"), SENTINEL]);
    let mut m = Mcp::start(&h);
    m.text("search_my_brain", json!({"query": SENTINEL}));
    m.text("capture", json!({"text": SENTINEL, "title": SENTINEL}));
    drop(m);
    let db = rusqlite::Connection::open(h.home.path().join("data/activity.db")).unwrap();
    let columns: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_info('counts') ORDER BY cid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, vec!["day", "kind", "door", "n"]);
    let rows: Vec<String> = db
        .prepare("SELECT day || ' ' || kind || ' ' || door || ' ' || n FROM counts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(rows.len() >= 5, "{rows:?}");
    for row in &rows {
        assert!(!row.contains(SENTINEL), "{row}");
    }
}

#[test]
fn a_count_that_cannot_be_written_never_fails_the_action() {
    let h = indexed(&[("a.md", "mailbox\n")]);
    let store = h.home.path().join("data/activity.db");
    let _ = fs::remove_file(&store);
    fs::create_dir_all(&store).unwrap();
    for args in [
        vec!["search", "mailbox"],
        vec!["search", "zeppelin"],
        vec!["add", "still saved"],
    ] {
        let out = h.brain().args(&args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("was not saved"),
            "{args:?}"
        );
    }
    let mut m = Mcp::start(&h);
    let (text, err) = m.text("capture", json!({"text": "kept anyway"}));
    assert!(!err, "{text}");
    assert_eq!(h.inbox().len(), 2);
}

// The daily send. Every test pins the clock with --now; today is 2026-11-25.

const TODAY: &str = "2026-11-25";

/// An instant on `day` that is the same date from UTC to UTC+13.
fn at(day: &str) -> String {
    format!("{day}T01:00:00Z")
}

fn on(h: &Home, day: &str, args: &[&str]) -> String {
    let now = at(day);
    let mut all = vec!["--now", now.as_str()];
    all.extend_from_slice(args);
    h.run(&all)
}

/// One count on each of `days`.
fn counted(days: &[&str]) -> Home {
    let h = Home::new();
    h.init();
    for day in days {
        on(&h, day, &["add", "a note"]);
    }
    h
}

fn send(h: &Home) -> String {
    on(h, TODAY, &["usage", "send"])
}

#[derive(Clone, Debug)]
struct Received {
    body: Vec<u8>,
    signature: String,
}

impl Received {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn day(&self) -> String {
        self.json()["day"].as_str().unwrap().to_string()
    }
}

enum Answer {
    Status(u16),
    Silent,
    /// 307 to this URL, which keeps the method and the body.
    Redirect(String),
}

/// A usage endpoint on a local port that keeps every request it is sent.
struct Endpoint {
    url: String,
    got: Arc<Mutex<Vec<Received>>>,
    /// How long a sender left unanswered waited before it hung up.
    waited: mpsc::Receiver<Duration>,
}

impl Endpoint {
    fn start(answer: Answer) -> Endpoint {
        Endpoint::with(answer, |_| {})
    }

    /// `during` runs when a request has arrived and before it is answered.
    fn with(answer: Answer, during: impl Fn(&Received) + Send + 'static) -> Endpoint {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/day", listener.local_addr().unwrap());
        let got = Arc::new(Mutex::new(Vec::new()));
        let kept = got.clone();
        let (hung_up, waited) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let r = read_request(&stream);
                during(&r);
                kept.lock().unwrap().push(r);
                match &answer {
                    Answer::Status(code) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {code} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        );
                    }
                    Answer::Silent => {
                        let (arrived, hung_up) = (Instant::now(), hung_up.clone());
                        // Nothing more is coming, so the read ends when the sender gives up:
                        // with a close, or on some systems a reset. On its own thread, so
                        // another request would still be taken and counted.
                        std::thread::spawn(move || {
                            let _ = stream.read_to_end(&mut Vec::new());
                            let _ = hung_up.send(arrived.elapsed());
                        });
                    }
                    Answer::Redirect(to) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 307 X\r\nlocation: {to}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        );
                    }
                }
            }
        });
        Endpoint { url, got, waited }
    }

    fn got(&self) -> Vec<Received> {
        self.got.lock().unwrap().clone()
    }

    fn point(&self, h: &Home) {
        h.usage(&format!("send = true\nendpoint = \"{}\"", self.url));
    }
}

fn read_request(stream: &TcpStream) -> Received {
    let mut r = BufReader::new(stream);
    let (mut length, mut signature) = (0, String::new());
    loop {
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap_or((line, ""));
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "brain-signature" => signature = value.trim().to_string(),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    r.read_exact(&mut body).unwrap();
    Received { body, signature }
}

/// The ledger as stored: `(day, body, signature, outcome)`, oldest first.
fn ledger(h: &Home) -> Vec<(String, Vec<u8>, String, String)> {
    let db = rusqlite::Connection::open(h.home.path().join("data/activity.db")).unwrap();
    db.prepare("SELECT day, body, signature, outcome FROM sends ORDER BY day")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn today() -> jiff::civil::Date {
    TODAY.parse().unwrap()
}

#[test]
fn a_usage_report_holds_only_counts() {
    let day = "2026-11-24";
    // The gap's question holds the sentinel, so it is asked before any note holds it.
    let h = indexed(&[]);
    on(
        &h,
        day,
        &["search", &format!("{SENTINEL} nowhere zeppelin")],
    );
    h.put(
        &format!("{SENTINEL}/{SENTINEL}.md"),
        &format!(
            "---\ntitle: {SENTINEL}\nowner: robin\nreview: 2020-01-01\n---\n{SENTINEL} body\n"
        ),
    );
    h.run(&["index"]);
    on(&h, day, &["search", SENTINEL]);
    on(
        &h,
        day,
        &["add", SENTINEL, "--title", &format!("{SENTINEL} one")],
    );
    on(
        &h,
        day,
        &["add", SENTINEL, "--title", &format!("{SENTINEL} two")],
    );
    let inbox = h.inbox();
    on(
        &h,
        day,
        &["inbox", "keep", &format!("0-inbox/{}", inbox[0])],
    );
    on(
        &h,
        day,
        &["inbox", "skip", &format!("0-inbox/{}", inbox[1])],
    );
    on(
        &h,
        day,
        &["correct", &format!("{SENTINEL}/{SENTINEL}.md"), SENTINEL],
    );
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    assert_eq!(got.len(), 1, "{got:?}");
    let body = String::from_utf8(got[0].body.clone()).unwrap();
    assert!(!body.contains(SENTINEL), "{body}");
    let v = got[0].json();
    let workspace = v["workspace"].as_str().unwrap();
    assert_eq!(workspace.len(), 43, "{body}");
    let mut allowed: Vec<&str> = Event::ALL.iter().map(|e| e.as_str()).collect();
    allowed.extend(Door::ALL.iter().map(|d| d.as_str()));
    allowed.extend([day, env!("CARGO_PKG_VERSION"), workspace]);
    fn leaves<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| leaves(x, out)),
            Value::Object(o) => o.values().for_each(|x| leaves(x, out)),
            leaf => out.push(leaf),
        }
    }
    let mut all = Vec::new();
    leaves(&v, &mut all);
    for leaf in all {
        match leaf {
            Value::Number(_) | Value::Null => {}
            Value::String(s) => assert!(allowed.contains(&s.as_str()), "{s} in {body}"),
            other => panic!("{other} in {body}"),
        }
    }
    let events: Vec<&str> = v["counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["event"].as_str().unwrap())
        .collect();
    for event in [
        "question",
        "gap_logged",
        "capture_written",
        "capture_kept",
        "capture_skipped",
        "correction",
        "notes_due",
    ] {
        assert!(events.contains(&event), "{event}: {body}");
    }
    assert_eq!(v["people"], 1, "{body}");
}

#[test]
fn a_report_verifies_with_the_install_key() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    assert_eq!(got.len(), 1);
    let report = accept(&got[0].body, &got[0].signature, today()).unwrap();
    assert_eq!(report.day.to_string(), "2026-11-24");
}

#[test]
fn every_send_is_ledgered_byte_for_byte() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    let rows = ledger(&h);
    assert_eq!(rows.len(), 1);
    let (day, body, signature, outcome) = &rows[0];
    assert_eq!((day.as_str(), outcome.as_str()), ("2026-11-24", "sent"));
    assert_eq!(&got[0].body, body);
    assert_eq!(&got[0].signature, signature);
    let shown = on(&h, TODAY, &["usage"]);
    let lines: Vec<&str> = shown.lines().collect();
    let row = lines
        .iter()
        .position(|l| l.starts_with("2026-11-24  ") && l.ends_with("  sent"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert_eq!(lines[row + 1].as_bytes(), body.as_slice(), "{shown}");
    assert_eq!(lines[row + 2], format!("signature {signature}"), "{shown}");
}

#[test]
fn switched_off_sends_nothing() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    h.run(&["usage", "off"]);
    send(&h);
    assert!(e.got().is_empty());
    assert!(ledger(&h).is_empty());
}

#[test]
fn days_counted_while_off_are_never_sent() {
    let h = counted(&["2026-11-19"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    on(&h, "2026-11-20", &["usage", "off"]);
    on(&h, "2026-11-21", &["add", "a note"]);
    on(&h, "2026-11-22", &["usage", "on"]);
    on(&h, "2026-11-23", &["add", "a note"]);
    send(&h);
    let sent: Vec<String> = e.got().iter().map(Received::day).collect();
    assert_eq!(
        sent,
        ["2026-11-23", "2026-11-24"],
        "a day even partly off never leaves"
    );
}

#[test]
fn a_send_that_finds_sending_off_holds_back_the_days_before() {
    let h = counted(&["2026-11-21"]);
    on(&h, "2026-11-22", &["usage", "send"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let sent: Vec<String> = e.got().iter().map(Received::day).collect();
    assert_eq!(sent, ["2026-11-23", "2026-11-24"]);
}

/// Runs `brain daemon` at today's fixed clock until it prints its send line, and returns its
/// stderr.
fn daemon_until_sent(h: &Home) -> String {
    let err = h.home.path().join("daemon.err");
    let mut d = std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
        .args(["--now", &at(TODAY), "daemon"])
        .env("BRAIN_HOME", h.home.path())
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("LL_MODELS_DIR")
        .stderr(fs::File::create(&err).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let text = loop {
        let text = fs::read_to_string(&err).unwrap_or_default();
        if text.contains("usage: ") || Instant::now() > deadline {
            break text;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = d.kill();
    let _ = d.wait();
    assert!(text.contains("usage: "), "{text}");
    text
}

#[test]
fn usage_off_stops_a_running_daemon() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(503));
    e.point(&h);
    daemon_until_sent(&h);
    assert_eq!(e.got().len(), 1);
    assert_eq!(ledger(&h)[0].3, "failed: 503");
    h.run(&["usage", "off"]);
    daemon_until_sent(&h);
    assert_eq!(
        e.got().len(),
        1,
        "a failed day is not resent once sending is off"
    );
}

#[test]
fn a_silent_endpoint_times_out() {
    let h = counted(&["2026-11-23", "2026-11-24"]);
    let e = Endpoint::start(Answer::Silent);
    e.point(&h);
    send(&h);
    // Timed from the request's arrival to the hang up, so a slow machine's own start and writes
    // are not counted. The sender allows ten seconds; the HTTP client's own default is thirty.
    let waited = e.waited.recv_timeout(Duration::from_secs(60)).unwrap();
    assert!(waited < Duration::from_secs(15), "{waited:?}");
    let rows = ledger(&h);
    assert_eq!(rows.len(), 1, "later days are not tried");
    assert_eq!(rows[0].0, "2026-11-23");
    assert_eq!(rows[0].3, "failed: TimedOut");
    assert_eq!(e.got().len(), 1);
}

#[test]
fn a_busy_endpoint_gets_the_day_again() {
    for code in [408, 429, 503] {
        let h = counted(&["2026-11-24"]);
        let e = Endpoint::start(Answer::Status(code));
        e.point(&h);
        send(&h);
        assert_eq!(ledger(&h)[0].3, format!("failed: {code}"));
        send(&h);
        assert_eq!(e.got().len(), 2, "{code}");
    }
}

#[test]
fn a_redirect_is_never_followed() {
    let h = counted(&["2026-11-24"]);
    let elsewhere = Endpoint::start(Answer::Status(204));
    let e = Endpoint::start(Answer::Redirect(elsewhere.url.clone()));
    e.point(&h);
    send(&h);
    assert_eq!(e.got().len(), 1);
    assert!(elsewhere.got().is_empty(), "the body reached another URL");
    assert_eq!(ledger(&h)[0].3, "failed: 307");
}

#[test]
fn a_refused_day_is_never_resent() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(422));
    e.point(&h);
    send(&h);
    assert_eq!(ledger(&h)[0].3, "refused: 422");
    send(&h);
    assert_eq!(e.got().len(), 1);
}

#[test]
fn a_day_before_the_first_count_is_never_sent() {
    let h = counted(&["2026-11-23"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let days: Vec<String> = e.got().iter().map(Received::day).collect();
    assert_eq!(days, ["2026-11-23", "2026-11-24"]);
    let quiet = e.got()[1].json();
    assert_eq!(quiet["people"], 0, "{quiet}");
}

#[test]
fn the_ledger_row_exists_before_the_request() {
    let h = counted(&["2026-11-24"]);
    let db = h.home.path().join("data/activity.db");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    let e = Endpoint::with(Answer::Status(204), move |r| {
        let c = rusqlite::Connection::open(&db).unwrap();
        let row: Option<Vec<u8>> = c
            .query_row("SELECT body FROM sends WHERE day = '2026-11-24'", [], |x| {
                x.get(0)
            })
            .ok();
        kept.lock()
            .unwrap()
            .push(row.as_deref() == Some(r.body.as_slice()));
    });
    e.point(&h);
    send(&h);
    assert_eq!(*seen.lock().unwrap(), [true]);
}

#[test]
fn two_senders_send_one_body() {
    let h = counted(&["2026-11-23", "2026-11-24"]);
    let e = Endpoint::with(Answer::Status(204), |_| {
        std::thread::sleep(Duration::from_millis(300))
    });
    e.point(&h);
    let spawn = || {
        std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
            .args(["--now", &at(TODAY), "usage", "send"])
            .env("BRAIN_HOME", h.home.path())
            .spawn()
            .unwrap()
    };
    let (mut a, mut b) = (spawn(), spawn());
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    let days: Vec<String> = e.got().iter().map(Received::day).collect();
    assert_eq!(days, ["2026-11-23", "2026-11-24"]);
}

#[test]
fn today_is_never_sent() {
    let h = counted(&[TODAY]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    assert!(e.got().is_empty());
    assert!(ledger(&h).is_empty());
}

#[test]
fn a_day_already_sent_is_never_sent_again() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    send(&h);
    assert_eq!(e.got().len(), 1);
}

#[test]
fn an_offline_week_sends_each_day_once_when_back() {
    let days = [
        "2026-11-17",
        "2026-11-18",
        "2026-11-19",
        "2026-11-20",
        "2026-11-21",
        "2026-11-22",
        "2026-11-23",
        "2026-11-24",
    ];
    let h = counted(&days);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    h.usage(&format!(
        "send = true\nendpoint = \"http://127.0.0.1:{port}/v1/day\""
    ));
    send(&h);
    let rows = ledger(&h);
    assert_eq!(rows.len(), 1, "the run stops at the first failure");
    assert_eq!(rows[0].0, "2026-11-18", "seven days back, no further");
    assert!(rows[0].3.starts_with("failed: "), "{}", rows[0].3);
    let first = rows[0].1.clone();
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    let sent: Vec<String> = got.iter().map(Received::day).collect();
    assert_eq!(sent, days[1..]);
    assert_eq!(got[0].body, first);
    send(&h);
    assert_eq!(e.got().len(), 7);
}

#[cfg(unix)]
#[test]
fn the_install_key_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let key = h.home.path().join("data/usage.key");
    assert_eq!(fs::read(&key).unwrap().len(), 32);
    let mode = fs::metadata(&key).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}

#[test]
fn a_day_of_brains_own_errors_has_no_people() {
    let h = counted(&["2026-11-23"]);
    let down = Endpoint::start(Answer::Status(503));
    down.point(&h);
    on(&h, "2026-11-24", &["usage", "send"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let quiet = e.got()[1].json();
    assert_eq!(quiet["day"], "2026-11-24");
    assert_eq!(quiet["counts"][0]["event"], "error_send", "{quiet}");
    assert_eq!(quiet["people"], 0, "{quiet}");
}

#[test]
fn the_install_key_is_stable_across_runs() {
    let h = counted(&["2026-11-23"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    on(&h, "2026-11-24", &["usage", "send"]);
    send(&h);
    let got = e.got();
    assert_eq!(got.len(), 2);
    let key: [u8; 32] = fs::read(h.home.path().join("data/usage.key"))
        .unwrap()
        .try_into()
        .unwrap();
    let ours = brain_report::Report::new(
        &ed25519_dalek::SigningKey::from_bytes(&key),
        today(),
        "",
        0,
        vec![],
    )
    .workspace;
    for r in got {
        assert_eq!(r.json()["workspace"], ours.as_str());
    }
}

#[test]
fn a_short_install_key_is_not_replaced() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    let key = h.home.path().join("data/usage.key");
    fs::write(&key, b"0123456789").unwrap();
    let out = h
        .brain()
        .args(["--now", &at(TODAY), "usage", "send"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("usage.key") && err.contains("delete"), "{err}");
    assert_eq!(fs::read(&key).unwrap(), b"0123456789");
    assert!(e.got().is_empty());
    assert!(ledger(&h).is_empty());
}

#[test]
fn switching_off_mid_run_stops_the_next_day() {
    let h = counted(&["2026-11-23", "2026-11-24"]);
    let home = h.home.path().to_path_buf();
    let e = Endpoint::with(Answer::Status(204), move |_| {
        let off = std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
            .args(["usage", "off"])
            .env("BRAIN_HOME", &home)
            .output()
            .unwrap();
        assert!(off.status.success());
    });
    e.point(&h);
    send(&h);
    assert_eq!(e.got().len(), 1);
}

#[test]
fn a_wrong_endpoint_path_keeps_the_day() {
    for code in [403, 404, 405] {
        let h = counted(&["2026-11-24"]);
        let e = Endpoint::start(Answer::Status(code));
        e.point(&h);
        send(&h);
        assert_eq!(ledger(&h)[0].3, format!("failed: {code}"));
        send(&h);
        assert_eq!(e.got().len(), 2, "{code}");
    }
}

#[test]
fn a_second_sender_says_another_is_sending() {
    let h = counted(&["2026-11-24"]);
    let e = Endpoint::with(Answer::Status(204), |_| {
        std::thread::sleep(Duration::from_millis(1500))
    });
    e.point(&h);
    let spawn = || {
        std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
            .args(["--now", &at(TODAY), "usage", "send"])
            .env("BRAIN_HOME", h.home.path())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let (a, b) = (spawn(), spawn());
    let outs = [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
    let said: Vec<String> = outs
        .iter()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .collect();
    assert_eq!(
        said.iter()
            .filter(|s| s.contains("another brain is sending"))
            .count(),
        1,
        "{said:?}"
    );
}

#[test]
fn a_failed_day_shows_in_brain_usage_and_is_dropped_once_a_week_old() {
    let h = counted(&["2026-11-18", "2026-11-26"]);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    h.usage(&format!(
        "send = true\nendpoint = \"http://127.0.0.1:{port}/v1/day\""
    ));
    on(&h, "2026-11-19", &["usage", "send"]);
    let shown = h.run(&["usage"]);
    assert!(
        shown
            .lines()
            .any(|l| l.starts_with("2026-11-18") && l.contains("failed: ")),
        "{shown}"
    );
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    on(&h, "2026-11-27", &["usage", "send"]);
    let sent: Vec<String> = e.got().iter().map(Received::day).collect();
    assert_eq!(
        sent.first().map(String::as_str),
        Some("2026-11-20"),
        "{sent:?}"
    );
    assert!(!sent.contains(&"2026-11-18".to_string()), "{sent:?}");
}

#[test]
fn a_hook_payload_never_reaches_a_usage_report() {
    let day = "2026-11-24";
    let h = counted(&[day]);
    h.brain()
        .args(["hook", "prompt"])
        .write_stdin(format!(
            r#"{{"session_id":"{SENTINEL}","prompt":"{SENTINEL}"}}"#
        ))
        .assert()
        .success();
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(!String::from_utf8_lossy(&got[0].body).contains(SENTINEL));
}

#[test]
fn no_due_count_from_another_vault_is_sent() {
    let day = "2026-11-24";
    let h = counted(&[day]);
    h.put(
        "due.md",
        "---\nowner: robin\nreview: 2020-01-01\n---\nbody\n",
    );
    h.run(&["index"]);
    let next = tempfile::tempdir().unwrap();
    h.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        next.path().to_str().unwrap(),
    ]);
    let e = Endpoint::start(Answer::Status(204));
    e.point(&h);
    send(&h);
    let got = e.got();
    assert_eq!(got.len(), 1, "{got:?}");
    let body = String::from_utf8_lossy(&got[0].body).into_owned();
    assert!(!body.contains(Event::NotesDue.as_str()), "{body}");
}
