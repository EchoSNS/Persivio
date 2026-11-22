-- Manual queue priority (move up/down). Lower runs first; ties fall back to created_at.
ALTER TABLE tasks ADD COLUMN queue_order INTEGER NOT NULL DEFAULT 0;
