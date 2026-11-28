-- The verdict store: what each reflex made of each hook, and the labels a person gives it. It
-- is not a cache, and the trace of a run may be pruned long before a label on it is.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE verdicts (
    -- Never reused: a person labels a verdict by this number.
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reflex TEXT NOT NULL,
    version INTEGER NOT NULL,
    -- Which decider made it: `rule` at the hook, others scoring afterwards.
    backend TEXT NOT NULL,
    point TEXT NOT NULL,
    at INTEGER NOT NULL,
    session TEXT,
    run TEXT,
    input_text TEXT NOT NULL,
    fired INTEGER NOT NULL,
    confidence REAL NOT NULL,
    reason TEXT NOT NULL,
    acted INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    label TEXT CHECK (label IN ('right', 'wrong'))
);

CREATE INDEX verdicts_session ON verdicts(reflex, backend, session);

-- Whose text a verdict's row holds, so a note that goes takes its words with it.
CREATE TABLE verdict_sources (
    verdict_id INTEGER NOT NULL REFERENCES verdicts(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    PRIMARY KEY (verdict_id, source)
);

CREATE INDEX verdict_sources_source ON verdict_sources(source);

CREATE TABLE picks (
    verdict_id INTEGER NOT NULL REFERENCES verdicts(id) ON DELETE CASCADE,
    rank INTEGER NOT NULL,
    key TEXT NOT NULL,
    score REAL NOT NULL,
    text TEXT NOT NULL,
    passed INTEGER NOT NULL,
    label TEXT CHECK (label IN ('right', 'wrong')),
    PRIMARY KEY (verdict_id, rank)
);
