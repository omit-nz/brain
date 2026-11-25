//! What omit keeps of a report: the statements run on SQLite, as D1 runs them.

use brain_report::{Count, Door, Event, Refused, Report};
use brain_usage::{Param, Statement, statements, status};
use ed25519_dalek::SigningKey;
use jiff::civil::{Date, date};
use rusqlite::Connection;
use rusqlite::types::Value;

const RECEIVED: Date = date(2026, 11, 25);

fn report() -> Report {
    Report::new(
        &SigningKey::from_bytes(&[7; 32]),
        date(2026, 11, 24),
        "0.1.1",
        1,
        vec![
            Count {
                event: Event::Question,
                door: Some(Door::Cli),
                n: 3,
            },
            Count {
                event: Event::NotesDue,
                door: None,
                n: 2,
            },
        ],
    )
}

fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(include_str!("../../migrations/0001_usage.sql"))
        .unwrap();
    c
}

fn run(c: &Connection, statements: &[Statement]) {
    for s in statements {
        let params: Vec<Value> = s
            .params
            .iter()
            .map(|p| match p {
                Param::Text(t) => Value::Text(t.clone()),
                Param::Int(i) => Value::Integer(*i),
            })
            .collect();
        c.execute(s.sql, rusqlite::params_from_iter(params))
            .unwrap();
    }
}

fn rows(c: &Connection, sql: &str) -> Vec<String> {
    c.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn days(c: &Connection) -> Vec<String> {
    rows(
        c,
        "SELECT workspace || ' ' || day || ' ' || version || ' ' || people || ' ' || received_on FROM days",
    )
}

fn counts(c: &Connection) -> Vec<String> {
    rows(
        c,
        "SELECT day || ' ' || event || ' ' || door || ' ' || n FROM counts ORDER BY event",
    )
}

#[test]
fn the_statements_run_on_sqlite_and_hold_only_counts() {
    let c = db();
    let r = report();
    run(&c, &statements(&r, RECEIVED));
    assert_eq!(
        rows(
            &c,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"
        ),
        ["counts", "days"]
    );
    assert_eq!(
        rows(
            &c,
            "SELECT name FROM pragma_table_info('days') ORDER BY cid"
        ),
        ["workspace", "day", "version", "people", "received_on"]
    );
    assert_eq!(
        rows(
            &c,
            "SELECT name FROM pragma_table_info('counts') ORDER BY cid"
        ),
        ["workspace", "day", "event", "door", "n"]
    );
    assert_eq!(
        days(&c),
        [format!("{} 2026-11-24 0.1.1 1 2026-11-25", r.workspace)]
    );
    assert_eq!(
        counts(&c),
        ["2026-11-24 notes_due  2", "2026-11-24 question cli 3"]
    );
    assert_eq!(
        rows(&c, "SELECT DISTINCT workspace FROM counts"),
        [r.workspace]
    );
}

#[test]
fn a_resent_day_rewrites_the_same_rows() {
    let c = db();
    let s = statements(&report(), RECEIVED);
    run(&c, &s);
    let (d, n) = (days(&c), counts(&c));
    run(&c, &statements(&report(), date(2026, 11, 26)));
    assert_eq!(days(&c), d, "a day keeps the date it was first received");
    assert_eq!(counts(&c), n);
}

#[test]
fn a_half_applied_batch_is_completed_by_a_resend() {
    let clean = db();
    let s = statements(&report(), RECEIVED);
    run(&clean, &s);
    let c = db();
    run(&c, &s[..s.len() / 2]);
    run(&c, &s);
    assert_eq!(days(&c), days(&clean));
    assert_eq!(counts(&c), counts(&clean));
}

#[test]
fn refusals_map_to_statuses() {
    for (refused, code) in [
        (Refused::TooLarge, 413),
        (Refused::BadSignature, 401),
        (Refused::BadKey, 401),
        (Refused::Malformed, 422),
        (Refused::DayOutOfRange, 422),
        (Refused::TooMany, 422),
    ] {
        assert_eq!(status(&refused), code, "{refused:?}");
    }
}

#[test]
fn one_event_through_two_doors_keeps_both_rows() {
    let c = db();
    let mut r = report();
    r.counts.push(Count {
        event: Event::Question,
        door: Some(Door::Other),
        n: 4,
    });
    run(&c, &statements(&r, RECEIVED));
    assert_eq!(
        counts(&c),
        [
            "2026-11-24 notes_due  2",
            "2026-11-24 question cli 3",
            "2026-11-24 question other 4"
        ]
    );
}

#[test]
fn a_changed_resend_overwrites_n_and_version() {
    let c = db();
    let r = report();
    run(&c, &statements(&r, RECEIVED));
    let mut changed = r.clone();
    changed.version = "0.1.2".into();
    changed.counts[0].n = 5;
    run(&c, &statements(&changed, date(2026, 11, 26)));
    assert_eq!(
        days(&c),
        [format!("{} 2026-11-24 0.1.2 1 2026-11-25", r.workspace)]
    );
    assert!(
        counts(&c).contains(&"2026-11-24 question cli 5".to_string()),
        "{:?}",
        counts(&c)
    );
}

#[test]
fn the_receipt_day_is_the_utc_date() {
    // 2026-11-24T23:59:59.999Z and one millisecond later.
    let last = 1_795_564_799_999;
    assert_eq!(brain_usage::utc_date(last), Some(date(2026, 11, 24)));
    assert_eq!(brain_usage::utc_date(last + 1), Some(date(2026, 11, 25)));
}

#[test]
fn one_ipv6_network_is_one_rate_limit_key() {
    assert_eq!(brain_usage::limit_key("203.0.113.7"), "203.0.113.7");
    assert_eq!(
        brain_usage::limit_key("2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
        brain_usage::limit_key("2001:db8:1:2::1"),
        "addresses in one /64 share a key"
    );
    assert_ne!(
        brain_usage::limit_key("2001:db8:1:2::1"),
        brain_usage::limit_key("2001:db8:1:3::1")
    );
    assert_eq!(
        brain_usage::limit_key("::ffff:203.0.113.7"),
        "203.0.113.7",
        "an IPv4 address written as IPv6 keeps its own key"
    );
    assert_ne!(
        brain_usage::limit_key("::ffff:203.0.113.7"),
        brain_usage::limit_key("::ffff:203.0.113.8")
    );
    assert_eq!(brain_usage::limit_key(""), "");
}
