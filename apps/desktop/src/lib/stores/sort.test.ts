import { describe, expect, it } from 'vitest';
import { toRow, type TaskRow } from './reducer.ts';
import { DEFAULT_SORT, isSortKey, loadSort, sortRows } from './sort.ts';

function row(id: string, createdAt: number, total: number | null, downloaded = 0): TaskRow {
	return toRow({
		id,
		kind: 'http',
		name: id,
		source: 'https://example.com/',
		destDir: '/d',
		state: { type: 'paused' },
		total,
		downloaded,
		createdAt,
		completedAt: null,
		category: null
	});
}

const rows = [row('file 10', 1, 100, 90), row('File 2', 2, null), row('a', 3, 500, 5)];
const ids = (list: TaskRow[]) => list.map((r) => r.id);

describe('sortRows', () => {
	it('sorts by every key without changing its input', () => {
		expect(ids(sortRows(rows, 'newest'))).toEqual(['a', 'File 2', 'file 10']);
		expect(ids(sortRows(rows, 'oldest'))).toEqual(['file 10', 'File 2', 'a']);
		expect(ids(sortRows(rows, 'name'))).toEqual(['a', 'File 2', 'file 10']);
		expect(ids(sortRows(rows, 'size'))).toEqual(['a', 'file 10', 'File 2']);
		expect(ids(sortRows(rows, 'progress'))).toEqual(['file 10', 'a', 'File 2']);
		expect(ids(rows)).toEqual(['file 10', 'File 2', 'a']);
	});

	it('keeps the order of equal rows', () => {
		const same = [row('x', 5, 1), row('y', 5, 1)];
		expect(ids(sortRows(same, 'size'))).toEqual(['x', 'y']);
	});

	it('falls back to the default without storage', () => {
		expect(loadSort()).toBe(DEFAULT_SORT);
		expect(isSortKey('nonsense')).toBe(false);
	});
});
