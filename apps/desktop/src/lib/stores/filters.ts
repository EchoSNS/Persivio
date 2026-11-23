// Sidebar filters and search over the task list.

import type { TaskRow } from './reducer.ts';
import { isActive } from './reducer.ts';

export type Filter =
	| { type: 'all' | 'active' | 'queued' | 'completed' | 'failed' | 'torrents' }
	| { type: 'category'; name: string };

export const BUILTIN_FILTERS = [
	{ type: 'all', label: 'All' },
	{ type: 'active', label: 'Active' },
	{ type: 'queued', label: 'Queued' },
	{ type: 'completed', label: 'Completed' },
	{ type: 'failed', label: 'Failed' },
	{ type: 'torrents', label: 'Torrents' }
] as const;

export function matchesFilter(row: TaskRow, filter: Filter): boolean {
	switch (filter.type) {
		case 'all':
			return true;
		case 'active':
			return isActive(row.state);
		case 'queued':
			return row.state.type === 'queued' || row.state.type === 'paused';
		case 'completed':
			return row.state.type === 'completed';
		case 'failed':
			return row.state.type === 'failed' || row.state.type === 'cancelled';
		case 'torrents':
			return row.kind === 'torrent';
		case 'category':
			return row.category === filter.name;
	}
}

/** Case-insensitive search in the name and source. */
export function matchesSearch(row: TaskRow, search: string): boolean {
	const q = search.trim().toLowerCase();
	return q === '' || row.name.toLowerCase().includes(q) || row.source.toLowerCase().includes(q);
}

export function applyFilters(rows: TaskRow[], filter: Filter, search: string): TaskRow[] {
	if (filter.type === 'all' && search.trim() === '') return rows;
	return rows.filter((r) => matchesFilter(r, filter) && matchesSearch(r, search));
}

export function filterKey(filter: Filter): string {
	return filter.type === 'category' ? `category:${filter.name}` : filter.type;
}

/** Number of tasks per filter key, for the sidebar badges. */
export function countByFilter(rows: TaskRow[], categories: string[]): Map<string, number> {
	const filters: Filter[] = [
		...BUILTIN_FILTERS.map((f) => ({ type: f.type })),
		...categories.map((name) => ({ type: 'category' as const, name }))
	];
	return new Map(
		filters.map((f) => [filterKey(f), rows.reduce((n, r) => n + (matchesFilter(r, f) ? 1 : 0), 0)])
	);
}
