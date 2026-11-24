# ADR-025: Stored credentials, full disks, and stuck servers

- **Status:** Accepted
- **Date:** 2026-11-24

## Context
The Phase 11 review against the security checklist, and the soak test, found:

1. Cookies, custom headers and URL logins stayed in the database for as long as a finished
   task was listed, although only a running or resumable download needs them.
2. A full disk surfaced as "file system error: …", not retryable, so the user had to remove
   the task and lose its data.
3. If the end of a run could not be saved (full disk, locked database), the task stayed
   "Downloading" forever although nothing ran.
4. "Automatic retries" covered segment workers only. One temporary 503 during the probe, or
   a broken connection to a server without range support, failed the task.
5. The probe had no response timeout and ignored cancellation: with a server that accepts
   connections and never answers, a task could not be paused.

## Decision
1. **Secrets are erased when a task completes or is cancelled** (cookies, custom headers,
   referer, URL login), from memory and the database, and at startup for tasks ended by
   older versions. Failed and paused tasks keep them, because a retry needs them.
   SQLite runs with `secure_delete=ON`, so erased values do not survive in free pages; the
   write-ahead log is merged when the app closes. On Unix the data directory is mode 0700.
   - *Accepted:* while a download can still run, its cookies are stored unencrypted. Any
     program of the same user can read the browser's own cookies too, so encrypting with a
     key stored beside the data would not add protection. OS keychains are a later option.
2. **`DiskFull` is recognized** wherever an I/O error says so (`StorageFull`,
   `QuotaExceeded`, librqbit's error text). Space is checked before writing for every
   download of known size, including single-connection ones and torrents.
3. **User retry is wider than automatic retry** (`is_user_retryable`): disk, file system,
   torrent and checkpoint errors can be retried by hand after the user fixed the cause.
4. **`end_unsaved`**: if a run's end cannot be stored, the task is shown as failed and
   retryable in memory. After a restart, recovery finds it "running" and pauses it, which
   is also safe.
5. **Retries cover every phase.** The probe and single-connection transfers use the same
   back-off policy. Segmented transfers are *not* retried as a whole (their workers already
   are), so retries never multiply.
6. **Every wait for a response** is bounded by the idle timeout and ends on cancellation.

## Consequences
- A finished task cannot be downloaded again with its cookies; add it again instead.
- A permanently failing server is reported after the retries (by default about 15 s)
  instead of at once.
- Servers without range support restart from zero on a retry, as before on a manual one.
