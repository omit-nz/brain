//! One note: frontmatter that round-trips, what a note may disclose, ids and wikilinks.

mod disclosure;
mod frontmatter;
mod links;
mod note;

pub use disclosure::{Disclosure, Place, disclosure, listed_summary};
pub use frontmatter::{FmError, Frontmatter, Split, Value};
pub use links::wikilinks;
pub use note::{Kind, Note, ensure_id, reassign_id, rel_key};
