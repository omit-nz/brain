# What brain keeps about its own decisions

Each time Claude Code calls `brain hook`, brain records what it read and what it decided, so any
step can be read and replayed later on the same machine.

- `traces.db`, in brain's data folder: one run per hook, with each thing read (the hook's payload,
  which holds your prompt, and any note text) and each verdict. Secrets are scrubbed before
  anything is written. A run is dropped after 30 days unless it is kept.
- The same file holds a run for each pass of `brain memory follow` that changed something, with the
  full text of each memory it copied or updated.
- When a note is removed, or stops being shared, the runs and verdicts holding its text are purged
  at the next index pass.
- `verdicts.db`, beside it: one row per reflex per hook, with what it picked and any label you
  give it. A row with no label is dropped after 90 days.

Both files can be read by you alone, and neither is ever synced or sent. The usage report holds
counts only and reads neither. To remove everything brain has recorded, delete the two files and
their `-wal` and `-shm` neighbours.
