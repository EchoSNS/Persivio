# ADR-016: How librqbit is mapped onto Persivio

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
librqbit 9.1.0 (behind the `Protocol` trait, ADR-006) has its own model: a session that
owns every torrent, optional JSON persistence that restores and starts torrents at
launch, and no concept of seeding limits or sequential download. Persivio's engine owns
scheduling and state (ADR-002) and its SQLite database is the source of truth (ADR-012).

## Decision
1. **One shared session**, created lazily from the first torrent task's settings: listen
   port, DHT, UPnP, extra trackers, global download limit. Changing those settings takes
   effect after an app restart.
2. **No librqbit persistence or fastresume.** The DHT routing-table cache is off too (librqbit
   would write it to `%LOCALAPPDATA%qbit`, outside Persivio's folders); the cost is a
   few seconds of DHT bootstrap per launch. Persivio stores the `.torrent` bytes and the
   info hash in `torrent_state` (via `CheckpointSink::save_torrent`). A resume re-adds
   the torrent from those bytes, with no new metadata fetch for magnets, and librqbit
   re-verifies the pieces already on disk. A paused task is removed from the session
   (files kept), so the engine alone decides what runs.
3. **Two-step add.** First `list_only`: for magnets this is the metadata fetch
   (`FetchingMetadata`), and it gives the file list. Every path is then **validated before
   anything is written** (`validate_torrent_path`): empty, `.` or `..` components,
   separators, absolute paths or drive prefixes, control characters and bidi-override
   characters cause a rejection. Torrent names cannot be rewritten (librqbit writes the
   metainfo names), so unsafe torrents are refused rather than sanitized. Then the real add.
4. **Layout:** a single file goes in the destination directly; a multi-file torrent goes
   in its own folder (with an explicit `output_folder`, librqbit would otherwise spill
   the files into the destination). On a fresh start an existing file or folder with the
   same name is never reused: a de-duplicated folder is used instead.
5. **Seeding limits are enforced by the adapter** (`seeding_done`): never seed, an upload
   ratio, or a seeding time. The first limit reached ends the task as `Completed`.
6. **Sequential download is not offered.** librqbit 9.1.0 has no such option (only a
   streaming API for reading). Faking it would mislead users.

## Consequences
- Resuming a large torrent re-reads its existing data to verify pieces, which costs disk I/O.
- Speed limits: the per-task limit maps to librqbit's per-torrent limit. The global limit
  is applied to the session at creation and does not yet follow runtime changes.
- A seeding torrent occupies a scheduler slot (`Seeding` is a running state). The CLI's
  `watch` does not wait for seeding to end.
- Tests use a real second librqbit session (`pv_torrent::testing::LocalSeeder`, feature
  `testing`), so other crates can test torrents without depending on librqbit.
- Defense in depth, observed in tests: librqbit itself already rejects `..` path
  components while decoding; Persivio's validator additionally rejects names that
  librqbit accepts, such as bidi-override spoofing.
