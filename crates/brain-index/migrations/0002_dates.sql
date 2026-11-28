-- What a note says about its own age and the notes it replaces, as written; both are resolved when
-- searching, so a rename or the day arriving needs no reread. `said` holds the candidate dates, space
-- separated, in the order they are tried. `notes_shared` selects every column, so it has these too.
ALTER TABLE notes ADD COLUMN said TEXT;
ALTER TABLE notes ADD COLUMN corrects TEXT;
ALTER TABLE notes ADD COLUMN supersedes TEXT;
