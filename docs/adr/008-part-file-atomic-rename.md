# ADR-008: .part file and atomic rename on completion

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Users and other programs (antivirus, media indexers, file managers) may open a file as soon as it
appears under its final name. A half-written file under the final name looks complete but is corrupt.

## Decision
Downloads are written to `<name>.part` in the destination directory and renamed to the final name
only after all bytes are written, synced and (optionally) verified.

## Consequences
- The final name only ever refers to a complete file; a rename within one directory is atomic.
- Windows may briefly lock the file (e.g. antivirus scanning) and fail the rename, so the rename
  is retried with backoff and reports a clear error if it keeps failing.
- Name collisions are resolved (de-duplicated) at rename time as well as at task creation.
