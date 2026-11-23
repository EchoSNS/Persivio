-- Files of a task as resolved from metadata (JSON array of pv_core::FileEntry). Kept so
-- the UI can show a torrent's files after a restart, without re-fetching metadata.
ALTER TABLE tasks ADD COLUMN files_json TEXT;
