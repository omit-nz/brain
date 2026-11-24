-- One row per note file. `id` is the index's own key and changes on a rebuild; `note_uuid` is the
-- note's id from its frontmatter, NULL when none can be written (broken frontmatter): such a note
-- is searchable by its owner and never synced. `embedded` is 0 pending, 1 done, -1 failed on its
-- own (the reason is in `skipped`).
CREATE TABLE notes (
    id INTEGER PRIMARY KEY,
    note_uuid TEXT UNIQUE,
    path TEXT NOT NULL UNIQUE,
    hash BLOB NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    title TEXT NOT NULL,
    tags TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('knowledge', 'rule', 'history')),
    place TEXT NOT NULL CHECK (place IN ('inbox', 'private', 'kept')),
    disclosure TEXT NOT NULL CHECK (disclosure IN ('shared', 'listed', 'private')),
    owner TEXT,
    review TEXT,
    embedded INTEGER NOT NULL DEFAULT 0
);

-- The text FTS5 indexes, one row per note under the same id.
CREATE TABLE notes_content (
    id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    tags TEXT NOT NULL,
    body TEXT NOT NULL
);

CREATE VIRTUAL TABLE notes_fts USING fts5(
    title, tags, body,
    content = 'notes_content',
    content_rowid = 'id',
    tokenize = 'porter unicode61 remove_diacritics 1'
);

CREATE TRIGGER notes_content_ai AFTER INSERT ON notes_content BEGIN
    INSERT INTO notes_fts(rowid, title, tags, body)
    VALUES (new.id, new.title, new.tags, new.body);
END;

CREATE TRIGGER notes_content_ad AFTER DELETE ON notes_content BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, title, tags, body)
    VALUES ('delete', old.id, old.title, old.tags, old.body);
END;

CREATE TRIGGER notes_content_au AFTER UPDATE ON notes_content BEGIN
    INSERT INTO notes_fts(notes_fts, rowid, title, tags, body)
    VALUES ('delete', old.id, old.title, old.tags, old.body);
    INSERT INTO notes_fts(rowid, title, tags, body)
    VALUES (new.id, new.title, new.tags, new.body);
END;

-- What a colleague could see: the items table for a shared-only search, so the filter runs before
-- ranking and a private note never takes a place in the results.
CREATE VIEW notes_shared AS SELECT * FROM notes WHERE disclosure = 'shared';

CREATE TABLE embeddings (
    id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    data BLOB NOT NULL
);

CREATE TABLE links (
    source_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    UNIQUE (source_id, target)
);

-- Backlinks and broken-link reports look links up by target.
CREATE INDEX links_target ON links(target);

-- Files the last reindex could not take, and notes whose embedding failed on their own.
CREATE TABLE skipped (
    path TEXT PRIMARY KEY,
    why TEXT NOT NULL
);

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
