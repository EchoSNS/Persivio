-- How much of a torrent is on disk (shown before it runs again) and, after a clean stop,
-- the protocol's record of which pieces (ADR-023).
ALTER TABLE torrent_state ADD COLUMN downloaded INTEGER NOT NULL DEFAULT 0 CHECK (downloaded >= 0);
ALTER TABLE torrent_state ADD COLUMN have BLOB;

UPDATE torrent_state
SET downloaded = COALESCE(
    (SELECT total_size FROM tasks WHERE tasks.id = torrent_state.task_id AND tasks.state = 'completed'), 0);
