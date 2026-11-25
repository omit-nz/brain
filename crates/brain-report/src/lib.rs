//! What brain counts about its own use: kinds of event and the door they came through. Counts
//! carry no note, no query and no person, only these names and numbers.

use serde::{Deserialize, Serialize};

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
