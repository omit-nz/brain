/// `[[target]]` links as learning-loop reads them: the alias after `|` and the anchor after `#`
/// dropped, trimmed, lower-cased, de-duplicated, in first-seen order.
pub fn wikilinks(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        let target = inner
            .split('|')
            .next()
            .unwrap_or("")
            .split('#')
            .next()
            .unwrap_or("");
        let target = target.trim().to_lowercase();
        if !target.is_empty() && !out.contains(&target) {
            out.push(target);
        }
        rest = &after[end + 2..];
    }
    out
}
