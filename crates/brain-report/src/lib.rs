//! What brain counts about its own use: kinds of event and the door they came through. Counts
//! carry no note, no query and no person, only these names and numbers. A day's counts leave as
//! a `Report`, signed with the install's own key; `accept` is the endpoint's check of one.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use jiff::ToSpan;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// The one report version `accept` takes.
pub const VERSION: u32 = 1;
const MAX_BODY: usize = 65_536;
const MAX_COUNTS: usize = 500;
const MAX_N: u64 = 10_000_000;
const MAX_AGE_DAYS: i64 = 30;

/// One closed day of one install. `workspace` is the install's public key, URL-safe base64: it
/// tells days of one install apart and names no one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub v: u32,
    pub workspace: String,
    pub day: Date,
    pub version: String,
    pub people: u32,
    pub counts: Vec<Count>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Count {
    pub event: Event,
    pub door: Option<Door>,
    pub n: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    TooLarge,
    Malformed,
    BadKey,
    BadSignature,
    DayOutOfRange,
    TooMany,
}

impl Report {
    /// A version 1 report whose workspace is `key`'s public half, so it always matches the
    /// signature.
    pub fn new(
        key: &SigningKey,
        day: Date,
        version: &str,
        people: u32,
        counts: Vec<Count>,
    ) -> Report {
        Report {
            v: VERSION,
            workspace: URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes()),
            day,
            version: version.to_string(),
            people,
            counts,
        }
    }
}

/// The body as sent and the URL-safe base64 signature over exactly those bytes.
pub fn sign(report: &Report, key: &SigningKey) -> (Vec<u8>, String) {
    let body = serde_json::to_vec(report).expect("a report always serialises");
    let signature = URL_SAFE_NO_PAD.encode(key.sign(&body).to_bytes());
    (body, signature)
}

/// The endpoint's check: the body is verified as received, before anything in it is trusted.
pub fn accept(body: &[u8], signature: &str, today: Date) -> Result<Report, Refused> {
    if body.len() > MAX_BODY {
        return Err(Refused::TooLarge);
    }
    let report: Report = serde_json::from_slice(body).map_err(|_| Refused::Malformed)?;
    if report.v != VERSION {
        return Err(Refused::Malformed);
    }
    let key = URL_SAFE_NO_PAD
        .decode(&report.workspace)
        .ok()
        .and_then(|k| <[u8; 32]>::try_from(k).ok())
        .and_then(|k| VerifyingKey::from_bytes(&k).ok())
        .ok_or(Refused::BadKey)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .ok()
        .and_then(|s| <[u8; 64]>::try_from(s).ok())
        .map(|s| Signature::from_bytes(&s))
        .ok_or(Refused::BadSignature)?;
    key.verify_strict(body, &signature)
        .map_err(|_| Refused::BadSignature)?;
    let oldest = today
        .checked_sub(MAX_AGE_DAYS.days())
        .map_err(|_| Refused::DayOutOfRange)?;
    if report.day > today || report.day < oldest {
        return Err(Refused::DayOutOfRange);
    }
    if report.counts.len() > MAX_COUNTS || report.counts.iter().any(|c| c.n > MAX_N) {
        return Err(Refused::TooMany);
    }
    Ok(report)
}

macro_rules! names {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &[$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $s),+
                }
            }

            pub fn parse(s: &str) -> Option<$name> {
                match s {
                    $($s => Some($name::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

names! {
    /// A kind of thing that happened, counted per day.
    Event {
        Question = "question",
        AnswerKept = "answer_kept",
        CaptureWritten = "capture_written",
        CaptureKept = "capture_kept",
        CaptureSkipped = "capture_skipped",
        Correction = "correction",
        GapLogged = "gap_logged",
        GapFilled = "gap_filled",
        NotesDue = "notes_due",
        SearchUnder100ms = "search_under100ms",
        SearchUnder300ms = "search_under300ms",
        SearchUnder1s = "search_under1s",
        SearchOver1s = "search_over1s",
        ErrorIndex = "error_index",
        ErrorEmbed = "error_embed",
        ErrorWatch = "error_watch",
        ErrorSend = "error_send",
        ErrorUpdate = "error_update",
    }
}

names! {
    /// Where a question or a capture came in.
    Door {
        ClaudeCode = "claude_code",
        ClaudeDesktop = "claude_desktop",
        Cli = "cli",
        Other = "other",
    }
}
