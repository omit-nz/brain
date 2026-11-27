# What brain keeps about its own decisions

Each time Claude Code calls `brain hook`, brain records what it read and what it decided, so any
step can be read and replayed later on the same machine.

- `traces.db`, in brain's data folder: one run per hook, with each thing read (the hook's payload,
  which holds your prompt, and any note text) and each verdict. Secrets are scrubbed before
  anything is written. A run is dropped after 30 days, the next time brain opens the file, unless it
  is kept.
- The same file holds a run for each pass of `brain memory follow` that changed something, with the
  full text of each memory it copied or updated.
- When a note is removed, or stops being shared, the runs and verdicts holding its text are purged
  at the next index pass, labelled verdicts too. That is tried once: what a pass that was cut short
  or a busy store misses ages out with the rest.
- A followed memory that `brain memory follow` archived is set aside, not removed: archiving purges
  nothing. Deleting its archived file while brain still tracks it purges its runs and verdicts. A
  memory that returns with other words is copied under the same id, so removing that copy purges
  them too. A fresh copy under a new id, by an import or another followed folder, leaves the old
  file untracked, and deleting it then purges nothing. An id is archived when brain's archive
  records name it and no note in the index holds it. Anything that gives an agent or a person text
  from a run or a verdict must say which of its sources are archived, worked out when it is read.
  Nothing reads that text back yet.
- `verdicts.db`, beside it: one row per reflex per hook, with what it picked and any label you
  give it. The daemon drops a row with no label after 90 days.

Both files can be read by you alone, and neither is ever synced or sent. The usage report holds
counts only and reads neither. To remove everything brain has recorded, delete the two files and
their `-wal` and `-shm` neighbours.
