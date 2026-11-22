-- Initial schema (build plan section 6). STRICT tables make SQLite enforce column types.
-- Integers are i64: sizes/offsets beyond 2^63-1 bytes are rejected by pv-store, not wrapped.

CREATE TABLE tasks (
    id            TEXT PRIMARY KEY NOT NULL,              -- UUID v7, hyphenated
    kind          TEXT NOT NULL CHECK (kind IN ('http', 'torrent')),
    source_json   TEXT NOT NULL,                          -- pv_core::Source as JSON
    dest_dir      TEXT NOT NULL,
    file_name     TEXT,                                   -- requested, later the resolved name
    final_url     TEXT,
    state         TEXT NOT NULL,                          -- TaskStateKind::as_str()
    fail_reason   TEXT,                                   -- JSON {reason, retryable} when failed
    total_size    INTEGER CHECK (total_size >= 0),
    etag          TEXT,
    last_modified TEXT,
    connections   INTEGER CHECK (connections BETWEEN 1 AND 32),
    speed_limit   INTEGER CHECK (speed_limit > 0),
    checksum_json TEXT,
    category      TEXT,
    created_at    INTEGER NOT NULL,                       -- unix millis
    updated_at    INTEGER NOT NULL,
    completed_at  INTEGER,
    CHECK ((state = 'failed') = (fail_reason IS NOT NULL))
) STRICT;

CREATE INDEX tasks_state_idx ON tasks (state);

CREATE TABLE segments (
    task_id    TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    idx        INTEGER NOT NULL CHECK (idx >= 0),
    start      INTEGER NOT NULL CHECK (start >= 0),
    "end"      INTEGER NOT NULL,                          -- inclusive
    downloaded INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (task_id, idx),
    CHECK ("end" >= start),
    CHECK (downloaded BETWEEN 0 AND "end" - start + 1)
) STRICT;

CREATE TABLE torrent_state (
    task_id             TEXT PRIMARY KEY NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    info_hash           TEXT NOT NULL,
    selected_files_json TEXT NOT NULL DEFAULT '[]',
    resume_blob         BLOB
) STRICT;

CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL
) STRICT;
