# What brain keeps about its own decisions and the calls made of it

Each time Claude Code calls `brain hook`, brain records what it read and what it decided, so any
step can be read and replayed later on the same machine.

- `traces.db`, in brain's data folder: one run per hook, with each thing read (the hook's payload,
  which holds your prompt, and any note text) and each verdict. Secrets are scrubbed before
  anything is written. A run is dropped after 30 days, the next time brain opens the file, unless it
  is kept.
- The same file holds a run for each call of a tool an AI tool makes through `brain mcp`: the
  tool, the door it came by, when it was answered, what it asked, how long the reply was, whether
  it failed, and the ids of the notes the reply named. No reply is kept. A capture, a correction,
  a room post, their titles, the note a correction names and a room or session name that brain
  would refuse as one are kept as their length, and so is anything over 2,000 characters. A question is kept as asked, so it holds whatever words were put
  in it. A call refused before it reaches its tool is not recorded, nor is one brain fails inside,
  and one can be missed when the store stays busy or brain is ended. `brain calls` reads them
  back, and they age out and are purged as the other runs are.
- The same file holds a run for each pass of `brain memory follow` that changed something, with the
  full text of each memory it copied or updated.
- When a note is removed, or stops being shared, the runs and verdicts holding its text are purged
  at the next index pass, labelled verdicts too. That is tried once: what a pass that was cut short
  or a busy store misses ages out with the rest.
- A followed memory that `brain memory follow` archived is set aside, not removed: archiving purges
  nothing. brain tracks the file it would bring back, the latest one it archived for that project
  and file name, until a copy for them is in the vault again. Deleting a tracked file purges the
  memory's runs and verdicts, once two passes have found it gone. Deleting any other file under
  `_archive` purges nothing. A memory that returns with other words is copied under the same id, so
  removing that copy can purge the old words' runs too. An id is archived when brain's archive
  records name it and no note in the index holds it. Anything that gives an agent or a person text
  from a run or a verdict must say which of its sources are archived, worked out when it is read,
  as `brain calls --json` does (the plain list says how many). Nothing else reads that text back yet.
- `verdicts.db`, beside it: one row per reflex per hook, with what it picked and any label you
  give it. The daemon drops a row with no label after 90 days.

Both files can be read by you alone, and neither is ever synced or sent. The usage report holds
counts only and reads neither. To remove everything brain has recorded, delete the two files and
their `-wal` and `-shm` neighbours.
