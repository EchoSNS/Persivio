# ADR-015: Scheduler slots counted by the actor, not a Semaphore

- **Status:** Accepted
- **Date:** 2026-11-23

## Context
The build plan suggests a `tokio::sync::Semaphore` for the maximum number of concurrent
tasks. Our engine is an actor (ADR-002): one task already owns the queue and the set of
running tasks, and processes every command and completion in order. The limit must be
changeable at runtime from settings. Shrinking a `Semaphore` means acquiring and
forgetting permits, which waits while tasks are running and is easy to get subtly wrong.

## Decision
The actor starts queued tasks in `schedule()` while `running.len() < max_concurrent_tasks`.
`schedule()` runs after every event that can free a slot or add work: add, resume, task
finished, settings changed. Lowering the limit never interrupts a running task; it only
stops new ones from starting until enough have finished.

## Consequences
- No extra synchronization: the count lives in the same single-threaded state as the
  queue, so "start the next task" can't race with "a task finished".
- A runtime limit change takes effect immediately for new starts.
- Manual priority (move up/down) is a reorder of the actor's `VecDeque`, persisted as
  `tasks.queue_order` (migration 0002).
- Test: `concurrency_limit_is_enforced` polls the engine while 5 tasks run with a
  limit of 2.
