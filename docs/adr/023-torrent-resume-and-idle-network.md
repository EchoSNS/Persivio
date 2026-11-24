# ADR-023: Torrent resume without re-reading, and no idle network use

- **Status:** Accepted (changes ADR-016 points 1 and 2)
- **Date:** 2026-11-24

## Context
Reported by the user with a 60 GB torrent: "fast downloading at first, then a new one
afterwards", again after every pause; and Malwarebytes reporting blocked addresses.

1. While librqbit checks existing data, `TorrentStats::progress_bytes` counts *hashed*
   bytes (good or bad, of all files). The adapter copied it into the download progress,
   so checking looked like a fast download that then restarted.
2. ADR-016 disabled librqbit's fast resume, so every resume re-read everything.
3. The librqbit session (port, DHT node) stayed open after the last torrent had finished.
   Torrent peers and DHT nodes are arbitrary addresses, some of them on blocklists.

## Decision
1. **A `Checking` task state** with its own percentage (`Progress::check_percent`).
   `ProgressSink::rebase` publishes what was found on disk without counting it as speed.
2. **A have map** (which pieces are on disk) is saved in `torrent_state.have`, only when a
   run stops cleanly, only **after every file was synced** (ADR-005), together with each
   file's size and modification time. It is cleared as soon as the torrent writes again.
   If any file differs on resume, the map is ignored and everything is checked.
3. librqbit can only *load* a piece map through its JSON session store. Persivio gives it
   a private scratch directory (`torrent-session` next to the database), **emptied at
   every session start**, so librqbit never restores or starts a torrent by itself.
   SQLite remains the source of truth (ADR-012). Nothing is left after a task is removed
   (foreign key cascade).
4. **Own torrent ids.** With persistence on, librqbit computes `max + 1` without a lock;
   two torrents added at once could get one id, and the second the first one's handle.
5. **The session closes 30 s after the last torrent run** and reopens on demand. It also
   picks up changed port/DHT/UPnP settings then. The slot stays locked while closing, so
   a new session cannot fight the old one for a fixed port.
6. One torrent, one task: adding a torrent that already runs fails with a clear message.
7. `Protocol::settings_changed` lets the running session follow the global speed limit.

## Consequences
- A clean pause of a torrent waits for its files to be synced.
- After a crash or power loss a full check runs, as before: the safe default.
- On load, librqbit samples some pieces of the map anyway and falls back to a full check
  if one is wrong.
- Security software still reports addresses while a torrent runs. That is inherent to
  BitTorrent: peers are strangers. Turning off DHT reduces it.

## Addendum: torrents with few peers (2026-11-24)
The soak test found torrents with a single peer standing still for minutes. librqbit now
and then loses the first message on a new connection (seen on the seeding side in about 1
of 100 local downloads). After the 10 s read timeout the peer is retried with a back-off of
10 s, then minutes, up to an hour; nothing shortens it.

Persivio restarts a torrent's networking (pause and unpause inside librqbit, which keeps
the pieces) when no peer has been connected for 1 minute, then 2, 4, … up to 10 minutes.
That retries known peers at once and asks trackers and the DHT again; the growing interval
keeps it polite to trackers.
