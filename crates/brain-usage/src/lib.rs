//! omit's usage endpoint. A report is checked by `brain_report::accept` and kept as rows of
//! counts with the date it arrived; nothing else of the request is stored. The statements are plain SQL with their
//! parameters, so the native tests run them on SQLite and the Worker shell runs them on D1.

use brain_report::{Refused, Report};
use jiff::civil::Date;

#[cfg(target_arch = "wasm32")]
mod shell;

pub struct Statement {
    pub sql: &'static str,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Text(String),
    Int(i64),
}

const DAY: &str = "INSERT INTO days(workspace, day, version, people, received_on)
     VALUES (?1, ?2, ?3, ?4, ?5)
     ON CONFLICT(workspace, day) DO UPDATE SET version = excluded.version, people = excluded.people";

const COUNT: &str = "INSERT INTO counts(workspace, day, event, door, n) VALUES (?1, ?2, ?3, ?4, ?5)
     ON CONFLICT(workspace, day, event, door) DO UPDATE SET n = excluded.n";

/// Upserts only, so a resend of the same bytes rewrites the same rows and completes a batch D1
/// applied in part. A day keeps the date it was first received.
pub fn statements(report: &Report, received: Date) -> Vec<Statement> {
    let workspace = || Param::Text(report.workspace.clone());
    let day = || Param::Text(report.day.to_string());
    let mut out = vec![Statement {
        sql: DAY,
        params: vec![
            workspace(),
            day(),
            Param::Text(report.version.clone()),
            Param::Int(i64::from(report.people)),
            Param::Text(received.to_string()),
        ],
    }];
    out.extend(report.counts.iter().map(|c| Statement {
        sql: COUNT,
        params: vec![
            workspace(),
            day(),
            Param::Text(c.event.as_str().into()),
            Param::Text(c.door.map_or("", |d| d.as_str()).into()),
            // accept caps n at 10,000,000, well inside i64.
            Param::Int(c.n as i64),
        ],
    }));
    out
}

/// The UTC date of an instant in milliseconds since the epoch, as the Worker's runtime gives it.
pub fn utc_date(millis: i64) -> Option<Date> {
    let t = jiff::Timestamp::from_millisecond(millis).ok()?;
    Some(t.to_zoned(jiff::tz::TimeZone::UTC).date())
}

/// The rate limit's key for a client's address: an IPv6 address cut to its /64, which one
/// machine or network usually holds whole, so it cannot take a fresh key per request; IPv4 as it
/// is, written as IPv4 even when it arrives as an IPv4-mapped IPv6 address.
pub fn limit_key(address: &str) -> String {
    match address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        _ => address.to_string(),
    }
}

pub fn status(refused: &Refused) -> u16 {
    match refused {
        Refused::TooLarge => 413,
        Refused::BadSignature | Refused::BadKey => 401,
        Refused::Malformed | Refused::DayOutOfRange | Refused::TooMany => 422,
    }
}
