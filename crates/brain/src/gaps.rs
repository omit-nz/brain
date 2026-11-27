//! Gaps: questions no note answered, logged against the nearest area and filled once a note
//! answers them.

use brain_index::activity::Activity;
use brain_index::{Hit, Index, Query, Scope, embed, is_gap};
use brain_platform::clock::Clock;
use brain_report::Event;
use ll_core::embed::EmbeddingProvider;

/// How many of the newest open gaps a recheck searches again.
const RECHECK: usize = 200;

/// The top hit's first folder, when it sits in one: the area a gap is nearest.
pub fn area(hits: &[Hit]) -> Option<String> {
    let top = hits.first()?;
    let (folder, _) = top.path.split_once('/')?;
    Some(folder.to_string())
}

/// A test that needs a long recheck sets this; release builds never read it.
fn pause() {
    if cfg!(debug_assertions)
        && let Ok(ms) = std::env::var("BRAIN_TEST_RECHECK_PAUSE_MS")
        && let Ok(ms) = ms.parse()
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// Searches the newest open gaps again and fills each one a note now answers. Returns how many
/// were filled.
pub fn recheck(
    index: &Index,
    activity: &Activity,
    provider: Option<&dyn EmbeddingProvider>,
    clock: &dyn Clock,
) -> anyhow::Result<usize> {
    let mut filled = 0;
    for gap in activity.open_gaps(RECHECK)? {
        pause();
        let q = Query {
            text: &gap.query,
            limit: 1,
            kinds: None,
            scope: Scope::Mine,
            rerank: embed::runtime().is_ok(),
        };
        if !is_gap(&index.search(&q, provider)?) {
            activity.fill_gap(gap.id, clock.now())?;
            activity.count(clock.today(), Event::GapFilled, Some(gap.door))?;
            filled += 1;
        }
    }
    Ok(filled)
}
