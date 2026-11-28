//! What brain recorded of a copy's bytes. brain rewriting a copy changes the file and the record,
//! which cannot be done together, so a record accepts both what is there and what is about to be.
//! The hashes are private here: nothing outside can compare against one of them alone.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hashes {
    #[serde(rename = "dst_hash")]
    written: String,
    /// The copy's hash before brain's last rewrite of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    was: Option<String>,
}

impl Hashes {
    /// Of bytes brain has just written.
    pub fn of(written: String) -> Hashes {
        Hashes { written, was: None }
    }

    /// Whether a file that hashes to `hash` is still as brain left it.
    pub fn accepts(&self, hash: &str) -> bool {
        self.written == hash || self.was.as_deref() == Some(hash)
    }

    /// Whether the two accept a hash in common.
    pub fn shares(&self, other: &Hashes) -> bool {
        self.accepts(&other.written) || other.was.as_deref().is_some_and(|was| self.accepts(was))
    }

    /// Records brain's rewrite of the file from `from` to `to`, before it is made. A record
    /// that accepts `from` only as what it was stays as it is: it accepts the file already, and
    /// the hash it would give up may be another file's. Says whether anything changed.
    pub fn rewrite(&mut self, from: &str, to: &str) -> bool {
        let moves = self.written == from;
        if moves {
            self.was = Some(std::mem::replace(&mut self.written, to.to_string()));
        }
        moves
    }

    /// Records the copy's bytes as they are now.
    pub fn settle(&mut self, hash: String) {
        self.written = hash;
    }
}
