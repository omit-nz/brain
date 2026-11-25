-- What omit keeps of brain's daily usage reports: counts per install and day, nothing else.
-- workspace is the install's public key; it names no person.

CREATE TABLE days (
    workspace TEXT NOT NULL,
    day TEXT NOT NULL,
    version TEXT NOT NULL,
    people INTEGER NOT NULL,
    received_on TEXT NOT NULL,
    PRIMARY KEY (workspace, day)
);

-- door is '' when a count has none.
CREATE TABLE counts (
    workspace TEXT NOT NULL,
    day TEXT NOT NULL,
    event TEXT NOT NULL,
    door TEXT NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (workspace, day, event, door)
);
