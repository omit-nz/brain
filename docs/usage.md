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

`brain usage` says whether sending is on and where to, then today's counts, one `kind  door  n` line each, then every report sent: its day, when it was made and what came of it, the report exactly as it left, and its signature.

Sending is on by default. `brain usage off` stops the sending and keeps counting on this machine only; `brain usage on` turns sending back on. The setting lives in `config.toml` under `[usage]`, and brain reads it before every send, so a running daemon stops at its next one.

A count that cannot be saved never stops what you were doing: brain says so in one line and carries on.

## What leaves, and when

One report per closed day, a JSON object with these fields and no others:

- `v`: the report format, 1.
- `workspace`: the public half of a key brain made for this install, kept in `usage.key` in its data folder (on macOS and Linux readable by you alone; on Windows it takes the folder's permissions, which are yours by default). It tells one install's days apart and names no one.
- `day`: the day counted, as `2026-11-24`.
- `version`: the brain version that made the report.
- `people`: 1 if brain was used that day, else 0. brain's own errors do not count as use.
- `counts`: each count of the day, as `event`, `door` (null when it has none) and `n`, plus `notes_due` when it is above 0: the number of your notes whose review date had come by that day, read from the index when the report is made, so a note reviewed since then is not in it.

The report is signed with the install's key, and the signature travels beside it in a `brain-signature` header, so omit can tell a report was not changed on the way.

The daemon sends at start and then hourly; today is never sent, only days that have ended. A run looks back seven days at most, from the first day anything was counted, so a laptop that was offline for a week sends each missed day once when it is back. Older days are never sent.

Before a report is sent, it is written to a usage ledger in `activity.db`, and a day is only ever sent as those ledgered bytes, so a retry repeats exactly what you can read in `brain usage`. A day omit accepted, or refused as a bad report (400, 401, 413 or 422), is never sent again; if brain stops after omit accepted a day but before it recorded that, the next run sends the same bytes again, and omit keeps them once. Any other answer, or none, leaves the day to be tried again at the next run, and that run stops at the first failure, waiting at most 10 seconds. A day that keeps failing holds back the days after it, and once it is more than seven days old it is never sent. brain follows no redirect, so a report reaches only the endpoint in your config.
