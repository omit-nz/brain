-- The verdict store: what each reflex made of each hook, and the labels a person gives it. It
-- is not a cache, and the trace of a run may be pruned long before a label on it is.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE verdicts (
    id INTEGER PRIMARY KEY,
    reflex TEXT NOT NULL,
    version INTEGER NOT NULL,
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

CREATE INDEX verdicts_session ON verdicts(reflex, session);

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
