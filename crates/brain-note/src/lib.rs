//! One note: frontmatter that round-trips, what a note may disclose, ids and wikilinks.

mod disclosure;
mod frontmatter;
pub mod gate;
mod links;
mod note;
mod vault;

pub use disclosure::{Disclosure, Place, RULES, disclosure, listed_summary};
pub use frontmatter::{FmError, Frontmatter, Split, Value, text_of};
pub use links::wikilinks;
pub use note::{Kind, Note, ensure_id, reassign_id, rel_key};
pub use vault::{Entry, Layout, MAX_NOTE_BYTES, Skip, Vault, Walk};
