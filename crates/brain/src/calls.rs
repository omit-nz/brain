//! What brain keeps of each tool call an AI makes, in the trace store: what was asked and how much
//! came back. No reply is kept, nor the words of a note being written. The notes a reply named are
//! the call's sources, so a note that is removed takes the calls that named it with it.

use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use brain_index::Index;
use brain_platform::dirs::Dirs;
use brain_reflex::TRACES;
use brain_reflex::trace::{Call, Error, Kind, Reply, Store};
use jiff::Timestamp;
use serde::Serializer;
use serde_json::{Value, json};

use crate::import::Archive;
use crate::room;

/// The name a tool call's run goes under, among the hooks' and the memory passes'.
const RUN: &str = "mcp";

/// How long a reply waits for its own record: another writer holding the store is waited on for
/// five seconds, and a reply is not.
const WAIT: Duration = Duration::from_secs(1);

/// The most characters an argument is kept with; longer is no question or name.
const LONGEST: usize = 2_000;

/// What is kept of words that are or become a note: how many there were.
fn count(text: &str) -> String {
    format!("{} characters", text.chars().count())
}

pub fn counted<S: Serializer>(text: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&count(text))
}

pub fn counted_if_given<S: Serializer>(text: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match text {
        Some(text) => counted(text, s),
        None => s.serialize_none(),
    }
}

/// A name is kept as given; what `check` says is none may be anything, a note's words among them.
fn name_if<S: Serializer>(
    name: &Option<String>,
    check: fn(&str) -> anyhow::Result<()>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match name {
        Some(text) if check(text).is_ok() => s.serialize_some(text),
        other => counted_if_given(other, s),
    }
}

pub fn room_name<S: Serializer>(name: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    name_if(name, room::check_room, s)
}

pub fn person_name<S: Serializer>(name: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    name_if(name, room::check_name, s)
}

/// Records one call and the reply it got, returning the writer when it is still at work. What
/// cannot be written is said and never changes the reply.
pub fn record(
    data: &Path,
    door: &'static str,
    tool: &'static str,
    mut args: Value,
    reply: &str,
    failed: bool,
) -> Option<JoinHandle<()>> {
    for value in args
        .as_object_mut()
        .into_iter()
        .flat_map(|a| a.values_mut())
    {
        if let Value::String(text) = value
            && text.chars().count() > LONGEST
        {
            *value = count(text).into();
        }
    }
    let data = data.to_path_buf();
    let request = json!({ "door": door, "args": args });
    let response = json!({ "bytes": reply.len(), "error": failed });
    let sources = named(reply);
    let (done, waiting) = mpsc::channel();
    let writer = thread::Builder::new().spawn(move || {
        let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
        let mut call = Call::new(Kind::Tool, tool, &request);
        // Written with the request, so no row is ever without the notes a purge finds it by.
        call.sources = &sources;
        let mut run = brain_reflex::record(&data, RUN);
        // A recording run answers with what it was given, so there is nothing to read back.
        let _ = run.call(call, || Ok(Reply::new(response)));
        if let Err(e) = run.finish() {
            eprintln!("brain: this {tool} call was not traced: {e}");
        }
        let _ = done.send(());
    });
    match writer {
        Ok(writer) => waiting.recv_timeout(WAIT).is_err().then_some(writer),
        Err(e) => {
            eprintln!("brain: this {tool} call was not traced: {e}");
            None
        }
    }
}

/// The notes and posts a reply named, as a purge knows them. A note's own text is escaped, so only
/// brain's own elements read like this.
fn named(reply: &str) -> Vec<String> {
    ["<note id=\"", "<post id=\""]
        .into_iter()
        .flat_map(|tag| reply.split(tag).skip(1))
        .filter_map(|rest| rest.get(..36)?.parse::<uuid::Uuid>().ok())
        .map(|id| format!("note:{id}"))
        .collect()
}

/// The last `n` tool calls recorded, newest first, each saying which of its notes are archived.
pub fn last(dirs: &Dirs, n: usize) -> anyhow::Result<Vec<Value>> {
    let path = dirs.data.join(TRACES);
    // Opening would make the file.
    if !path.exists() {
        return Ok(Vec::new());
    }
    let store = Store::open(&path)?;
    let archive = Archive::load(dirs)?;
    let held = dirs.data.join("index.db");
    // Opening would make an empty one, which holds no note.
    anyhow::ensure!(held.exists(), "no index here; run `brain index`");
    let index = Index::open(&held)?;
    // Archived: set aside by brain and held by no note now.
    let archived = |id: &str| -> anyhow::Result<bool> {
        Ok(match uuid::Uuid::parse_str(id) {
            Ok(uuid) if archive.names(id) => index.path_of(&uuid)?.is_none(),
            _ => false,
        })
    };
    let mut out = Vec::new();
    for run in store.runs()?.iter().filter(|r| r.name == RUN) {
        if out.len() == n {
            break;
        }
        let at = Timestamp::from_nanosecond(run.started_ns.into())?;
        // A prune or a purge may take a run while the list is read.
        let spans = match store.spans(&run.id) {
            Err(Error::NoSuchRun) => continue,
            spans => spans?,
        };
        // One call to a run; a run cut short before it holds none.
        for span in spans {
            let request = span.request.unwrap_or_default();
            let response = span.response.unwrap_or_default();
            let notes: Vec<&str> = span
                .sources
                .iter()
                .filter_map(|s| s.strip_prefix("note:"))
                .collect();
            let mut aside = Vec::new();
            for id in &notes {
                if archived(id)? {
                    aside.push(*id);
                }
            }
            out.push(json!({
                "at": at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "door": request["door"],
                "tool": span.name,
                "args": request["args"],
                "reply_bytes": response["bytes"],
                "error": response["error"],
                "notes": notes,
                "archived": aside,
            }));
        }
    }
    Ok(out)
}
