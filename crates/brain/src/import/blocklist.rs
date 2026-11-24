//! learning-loop's visibility rules, read from `<ll-config>/federation/config.json`. Matching is
//! learning-loop's: case-insensitive globs where `*` also crosses `/`, and the last match wins.
//! Only a rule that yields `private` matters here: on import a rule may withhold a note, never
//! share one.

use std::path::Path;

use anyhow::{Context, bail};
use globset::{GlobBuilder, GlobMatcher};

pub struct Blocklist {
    rules: Vec<(GlobMatcher, String)>,
}

impl Blocklist {
    pub fn load(ll_config: &Path) -> anyhow::Result<Blocklist> {
        let path = ll_config.join("federation").join("config.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading learning-loop's rules at {}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        let Some(rules) = v["visibility"]["rules"].as_array() else {
            bail!("{} has no visibility.rules list", path.display());
        };
        let mut out = Vec::new();
        for r in rules {
            let (Some(pattern), Some(tier)) = (r["pattern"].as_str(), r["tier"].as_str()) else {
                bail!(
                    "{}: a rule without a pattern and a tier: {r}",
                    path.display()
                );
            };
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("rule {pattern:?} is not a valid glob"))?;
            out.push((glob.compile_matcher(), tier.trim().to_string()));
        }
        Ok(Blocklist { rules: out })
    }

    /// Whether the last rule matching `path` (a vault path with `/`) says private.
    pub fn withholds(&self, path: &str) -> bool {
        self.rules
            .iter()
            .rev()
            .find(|(glob, _)| glob.is_match(path))
            .is_some_and(|(_, tier)| tier == "private")
    }
}
