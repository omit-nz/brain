-- The usage ledger: each closed day's report as signed, written before it is sent and resent
-- byte for byte. outcome is pending, sent, refused: <status> or failed: <status or error kind>.

CREATE TABLE sends (
    day TEXT PRIMARY KEY,
    at TEXT NOT NULL,
    body BLOB NOT NULL,
    signature TEXT NOT NULL,
    outcome TEXT NOT NULL
);
