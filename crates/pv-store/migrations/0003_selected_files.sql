-- Torrent file selection (JSON array of file indexes; NULL = all files).
ALTER TABLE tasks ADD COLUMN selected_files_json TEXT;
