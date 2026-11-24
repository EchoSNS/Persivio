// Sorting of the task list. Pure functions; the choice is remembered per machine.

import { percent } from '#lib/utils/format.ts';
import type { TaskRow } from './reducer.ts';

export const SORTS = [
	{ key: 'newest', label: 'Newest first' },
	{ key: 'oldest', label: 'Oldest first' },
	{ key: 'name', label: 'Name' },
	{ key: 'size', label: 'Largest first' },
	{ key: 'progress', label: 'Progress' }
] as const;

export type SortKey = (typeof SORTS)[number]['key'];

const STORAGE_KEY = 'persivio.sort';
export const DEFAULT_SORT: SortKey = 'newest';

const COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

const COMPARE: Record<SortKey, (a: TaskRow, b: TaskRow) => number> = {
	newest: (a, b) => b.createdAt - a.createdAt,
	oldest: (a, b) => a.createdAt - b.createdAt,
	name: (a, b) => COLLATOR.compare(a.name, b.name),
	// Unknown sizes go last.
	size: (a, b) => (b.total ?? -1) - (a.total ?? -1),
	progress: (a, b) => (percent(b.downloaded, b.total) ?? -1) - (percent(a.downloaded, a.total) ?? -1)
};

/** A sorted copy. Equal rows keep their order (the sort is stable). */
export function sortRows(rows: TaskRow[], key: SortKey): TaskRow[] {
	return rows.slice().sort(COMPARE[key]);
}

export function isSortKey(value: unknown): value is SortKey {
	return SORTS.some((s) => s.key === value);
}

export function loadSort(): SortKey {
	try {
		const v = localStorage.getItem(STORAGE_KEY);
		return isSortKey(v) ? v : DEFAULT_SORT;
	} catch {
		return DEFAULT_SORT;
	}
}

export function saveSort(key: SortKey): void {
	try {
		localStorage.setItem(STORAGE_KEY, key);
	} catch {
		// Remembered for this session only.
	}
}
