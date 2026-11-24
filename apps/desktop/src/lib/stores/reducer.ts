// Pure functions that apply engine events and progress batches to the task list.
// The Svelte store (tasks.svelte.ts) holds the state; these decide how it changes, and
// are tested without Tauri.

import type { EngineUiEvent, ProgressView, TaskStateView, TaskView } from '#lib/api/bindings.ts';

/** A task plus its live numbers (which only the progress channel provides). */
export type TaskRow = TaskView & {
	speedBps: number;
	etaSecs: number | null;
	connections: number;
	peers: number | null;
	uploadBps: number | null;
	/** While existing data is re-checked: percent done. */
	checkPercent: number | null;
};

/** States in which a task transfers data and receives progress. */
const ACTIVE: ReadonlySet<TaskStateView['type']> = new Set([
	'probing',
	'fetching_metadata',
	'checking',
	'downloading',
	'verifying',
	'seeding'
]);

export function isActive(state: TaskStateView): boolean {
	return ACTIVE.has(state.type);
}

export function toRow(task: TaskView): TaskRow {
	return {
		...task,
		speedBps: 0,
		etaSecs: null,
		connections: 0,
		peers: null,
		uploadBps: null,
		checkPercent: null
	};
}

/** Replaces the list with a fresh one from `listTasks`. */
export function fromList(tasks: TaskView[]): TaskRow[] {
	return tasks.map(toRow);
}

/** What the store should do after an event. */
export type EventResult = { rows: TaskRow[]; resync: boolean; warning: string | null };

/** Applies one engine event. Returns the same array when nothing changed. */
export function applyEvent(rows: TaskRow[], event: EngineUiEvent): EventResult {
	const result = (next: TaskRow[]): EventResult => ({ rows: next, resync: false, warning: null });
	switch (event.type) {
		case 'task_added':
			// Idempotent: the task may already be in a list loaded meanwhile.
			if (rows.some((r) => r.id === event.task.id)) return result(rows);
			return result([...rows, toRow(event.task)]);
		case 'state_changed':
			return result(
				update(rows, event.id, (row) => {
					const next: TaskRow = { ...row, state: event.state };
					if (!isActive(event.state)) {
						next.speedBps = 0;
						next.etaSecs = null;
						next.connections = 0;
						next.uploadBps = null;
					}
					if (event.state.type !== 'checking') next.checkPercent = null;
					if (event.state.type === 'completed') {
						if (row.total !== null) next.downloaded = row.total;
						next.completedAt ??= Date.now();
					}
					return next;
				})
			);
		case 'metadata_resolved':
			return result(
				update(rows, event.id, (row) => ({ ...row, name: event.name, total: event.size ?? row.total }))
			);
		case 'task_removed':
			return result(rows.filter((r) => r.id !== event.id));
		case 'warning':
			return { rows, resync: false, warning: event.message };
		case 'resync':
			return { rows, resync: true, warning: null };
	}
}

/** Applies a progress batch: the live numbers of every running task. */
export function applyProgress(rows: TaskRow[], batch: ProgressView[]): TaskRow[] {
	if (batch.length === 0) return rows;
	const byId = new Map(batch.map((p) => [p.id, p]));
	let changed = false;
	const next = rows.map((row) => {
		const p = byId.get(row.id);
		if (!p) return row;
		changed = true;
		return {
			...row,
			downloaded: p.downloaded,
			total: p.total ?? row.total,
			speedBps: p.speedBps,
			etaSecs: p.etaSecs,
			connections: p.connections,
			peers: p.peers,
			uploadBps: p.uploadBps,
			checkPercent: p.checkPercent
		};
	});
	return changed ? next : rows;
}

function update(rows: TaskRow[], id: string, change: (row: TaskRow) => TaskRow): TaskRow[] {
	const index = rows.findIndex((r) => r.id === id);
	const row = rows[index];
	if (row === undefined) return rows;
	const next = rows.slice();
	next[index] = change(row);
	return next;
}
