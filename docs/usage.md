# Usage counts

omit looks at usage, never at notes.

brain counts how it is used, as numbers only, so omit can see what is working and what is not. The counts live in `activity.db` in brain's data folder, on your machine.

## What is counted

Each count is a kind of event, the door it came through and a number, per day:

- questions asked of the brain, by front door (Claude Code, Claude Desktop, the CLI);
- answers that drew on a kept note;
- captures written, kept and skipped; corrections; gaps logged and gaps later filled;
- search speed, in four bands (under 100 ms, under 300 ms, under a second, longer);
- daemon errors by kind (indexing, embedding, watching the vault, sending, updating).

Later parts of brain add answers from a colleague's note, notes due and reviewed, recalled notes and reflex verdicts, and the version running.

## What is never counted

Note text, titles, search or prompt text, names, email addresses, file paths, app or window names, URLs and screen text. No count carries a person's identity. omit sees how many people were active, never who.

A question no note answered is kept as a gap, with its words, so brain can tell you when a note answers it later. Gaps stay in `activity.db` on your machine; no report reads them.

## Seeing the counts and switching sending off

`brain usage` says whether sending is on and where to, then today's counts, one `kind  door  n` line each.

Sending is on by default. `brain usage off` keeps the counts on this machine only; `brain usage on` turns sending back on. The setting lives in `config.toml` under `[usage]`.

Once a day, brain sends the counts of each closed day to omit's usage endpoint, signed with its own key, and keeps every send in a usage ledger beside the counts; `brain usage` shows each one byte for byte. A count that cannot be saved never stops what you were doing: brain says so in one line and carries on.
