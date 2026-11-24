# Hardening report (Phase 11, version 0.3.0)

Date: 2026-11-24. Machine: Windows 11, Ryzen 5 5600X, NVMe disk. All results below were
measured, not estimated; what was **not** verified is listed at the end.

## Findings

| # | Found by | Problem | Fix | Test |
|---|---|---|---|---|
| 1 | user report | Checking a torrent's existing data looked like a fast download that started over | `Checking` state, ADR-023 | `checking_existing_data_is_not_reported_as_downloading` |
| 2 | user report | Every torrent resume re-read all data | have map after a clean, synced stop | `clean_resumes_skip_the_full_check_until_the_files_change` |
| 3 | user report (blocked addresses) | BitTorrent network stayed active with no torrent running | session closes when idle | `the_session_closes_when_idle_and_reopens_when_needed` |
| 4 | review | Two tasks for one torrent shared its files | refused | `a_torrent_that_is_already_running_is_refused` |
| 5 | review | Cookies and headers stayed in the database after a download ended | erased, `secure_delete` (ADR-025) | `credentials_are_erased_from_disk_when_a_download_ends` |
| 6 | review | A full disk gave a vague error without Retry | `DiskFull`, space checks | `downloads_that_cannot_fit_are_refused_before_writing` and the small-disk tests |
| 7 | review | A task whose end could not be saved looked as if it still ran | `end_unsaved` | `a_task_whose_end_cannot_be_saved_is_shown_as_failed_and_can_be_resumed` |
| 8 | soak | One 503 while connecting failed a download despite "Automatic retries" | retries in every phase | `temporary_failures_are_retried_in_every_phase` |
| 9 | review of 8 | A server that never answers could not be paused and never timed out | bounded, cancellable waits | `a_server_that_never_answers_can_be_paused_and_times_out` |
| 10 | soak | A torrent with one peer could stand still for up to an hour | search again, ADR-023 addendum | `a_lone_peer_that_appears_later_is_found_without_the_long_back_off` |
| 11 | review | Data folder readable by other users on Unix | mode 0700 | `a_directory_can_be_opened_once_at_a_time` |
| 12 | UI review | File names vanished from the list with the details panel open | new row layout | checked by screenshots |

## Security checklist (build plan, section 11)

| Item | Status | Where |
|---|---|---|
| Server file names sanitized, final path contained | Done | `sanitize_filename`, `contained_path`; property tests, fuzz target |
| Torrent paths validated and contained | Done | `validate_torrent_path`; fuzz target |
| No redirects to non-HTTP(S) schemes | Done | `refuses_redirects_to_non_http_schemes`, ADR-017 |
| TLS with rustls, validation always on | Done | no option to disable it exists |
| Bridge: loopback, token, origin, size limit | Done | `apps/desktop/src-tauri/tests/bridge.rs`, ADR-021 |
| Least-privilege Tauri capabilities | Done | `capabilities/main.json`: no file system, shell or opener access |
| Content Security Policy | Done | `tauri.conf.json`; no inline scripts or styles |
| Credentials at rest only as needed, redacted in logs | Done; rest accepted | erased when a download ends; stored unencrypted while it can still run (accepted in ADR-025); `Debug` and network errors are redacted |
| Update artifacts signed and verified | Done | updater public key, ADR-022 |
| `cargo deny` in CI | Done | one documented, unreachable advisory (RUSTSEC-2026-0293) |
| Finished files never run automatically | Done | only "Show in folder"; Mark-of-the-Web on Windows |

`npm audit`: 0 vulnerabilities.

## Soak test

`crates/pv-engine/tests/soak.rs`, one hour, release build: rounds of 100 downloads (plain,
throttled, flaky, without ranges, torrents), 15 of them paused and resumed per round.

| | |
|---|---|
| Duration | 3612 s, 332 rounds |
| Downloads | 33,200, every SHA-256 correct |
| Memory (committed) | 51.2 MiB (first quarter) to 54.0 MiB (last quarter), peak 57.8 MiB |
| Result | Pass |

The first attempt failed after 10 minutes and led to finding 10. Remaining effect: a torrent
whose only peer goes silent repeatedly can still take minutes (70 of 332 rounds were slowed
by this, the worst took 182 s). One round was slowed by a test-suite build on the same
machine. The limit per round is 5 minutes.

## Fuzzing

Targets: `content_range`, `content_disposition`, `sanitize_filename`, `magnet`,
`torrent_file`; each runs 10 minutes in CI on Linux (libFuzzer needs nightly Rust).
`torrent_file` found an integer overflow in librqbit's parser within a minute (ADR-026);
the other four ran clean.

## Benchmarks

| Benchmark | Result |
|---|---|
| Local download, 1 connection | 590 MiB/s |
| 2 connections | 408 MiB/s |
| 4 connections | 358 MiB/s |
| 8 connections | 311 MiB/s |
| 16 connections | 330 MiB/s |
| Writer alone (write and sync 32 MiB) | 700 MiB/s |
| Engine, 200 paused tasks: list them | 73 µs |
| Engine, 200 paused tasks: start and stop | 6.3 ms |
| UI, 509 rows: scroll frame | median 7 ms, worst 8 ms |

More connections are slower *here* because nothing limits a single one; every connection
only adds work. They pay off when the server limits each connection: a separate test
requires at least 5 times the throughput with 8. Compared with version 0.1 (commit
`7845877`) alternately on the same machine: 565, 352 and 301 MiB/s for 1, 4 and 8
connections, so there is no regression.

## Coverage (lines, regions)

| Crate | Lines | Regions |
|---|---|---|
| pv-core | 99.4 % | 99.3 % |
| pv-store | 97.8 % | 81.6 % |
| pv-torrent | 93.6 % | 89.8 % |
| pv-http | 92.0 % | 89.9 % |
| pv-engine | 88.2 % | 84.1 % |

Measured with `cargo llvm-cov nextest` before finding 10 was fixed. Target: 80 %.

## Not verified

- The two tests that fill a disk *underneath* a running download need a small file system.
  They pass in CI on Linux and have not been run on Windows or macOS.
- The glass theme in WebKitGTK (Linux) and WKWebView (macOS).
- Power loss. Durability is tested by killing the process, not the machine.
