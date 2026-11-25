-- The activity store: what brain did, counted per day, and the questions no note answered.
-- Unlike the index it is not rebuilt from the vault, so it is never deleted with the index.

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One row per day, kind and door ('' when a count has no door). Kinds and doors are names from
-- brain-report; nothing here holds note or question text.
CREATE TABLE counts (
    day TEXT NOT NULL,
    kind TEXT NOT NULL,
    door TEXT NOT NULL DEFAULT '',
    n INTEGER NOT NULL,
    PRIMARY KEY (day, kind, door)
);

-- What was asked and found wanting. The question text stays on this machine: no report reads
-- this table.
CREATE TABLE gaps (
    id INTEGER PRIMARY KEY,
    at TEXT NOT NULL,
    door TEXT NOT NULL,
    query TEXT NOT NULL,
    area TEXT,
    filled_at TEXT
);

CREATE INDEX gaps_open ON gaps(at) WHERE filled_at IS NULL;
