# What a note discloses

A team brain is shared by default. Every note is shared unless its frontmatter says otherwise or it sits somewhere that is never shared. One function, `brain_note::disclosure`, makes this call, and every doubt withholds: with shared as the default, a path that failed open would publish a note against its author's intent.

A note gets one of three disclosures:

- **Shared**: colleagues can find and read it.
- **Listed**: colleagues can find it and see its title and a short scrubbed summary, but not read it.
- **Private**: it never leaves the person's machine.

## Where the note sits

- A note in the inbox is private. A note is shared only once its person keeps it.
- A note in the person's private folder is private, whatever its frontmatter says.
- A kept note anywhere else is decided by its frontmatter.

## What the frontmatter says

For a kept note, read the `visibility` line:

- No frontmatter, or frontmatter with no `visibility` key: shared.
- `shared` or `public`: shared.
- `listed`: listed.
- `private`: private.
- Anything else is private: a typo (`pubic`), an empty value, `yes`, `true`, a list (`[shared]`), a block (`|`), a mapping, a half-quoted value (`"public`).

Before the value is compared, balanced quotes are removed (`"shared"` and `'shared'` read as `shared`), case is ignored, and an unquoted value loses a trailing comment. As in YAML, a comment starts at a `#` that follows a space or tab: `shared  # for the team` reads as `shared`, but `shared#x` is one unknown value and withholds.

## When the frontmatter is in doubt

Each of these makes a kept note private:

- The frontmatter is broken: it opens with `---` and never closes.
- `visibility` is written twice.
- `visibility` is written any way other than plain `visibility:` at the start of a line: indented or under another key, quoted (`"visibility":`), with a space before the colon (`visibility :`), in another case (`Visibility:`), inside a flow mapping (`{visibility: private}`), or as an explicit `? visibility` key. A reader of plain keys would miss these, so they are never read as absent.

A fence with trailing spaces (`--- `) is still a fence, so a note whose editor left one keeps its `visibility` line.

## The listed summary

A listed note sends its title and the first paragraph of its body. The whole body is scrubbed of credentials first (API keys and tokens, private key blocks, passwords in URLs and `key = value` pairs), so a secret that a paragraph break would split is still caught. A private key block with no end line is removed to the end of the text. The paragraph is then cut to 300 characters at a word boundary, ending in `...` when cut.
