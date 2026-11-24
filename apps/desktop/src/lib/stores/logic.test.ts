import { describe, expect, it } from 'vitest';
import type { TaskView } from '#lib/api/bindings.ts';
import { actionFor } from '#lib/shortcuts.ts';
import { applyFilters, countByFilter, matchesFilter } from './filters.ts';
import { recordSpeeds, sparklinePath } from './history.ts';
import { toRow } from './reducer.ts';
import { EMPTY, click, move, prune, selectAll } from './selection.ts';

function row(id: string, state: TaskView['state'], extra: Partial<TaskView> = {}) {
	return toRow({
		id,
		kind: 'http',
		name: `${id}.bin`,
		source: `https://example.com/${id}`,
		destDir: '/d',
		state,
		total: null,
		downloaded: 0,
		createdAt: 0,
		completedAt: null,
		category: null,
		...extra
	});
}

describe('filters', () => {
	const rows = [
		row('a', { type: 'downloading' }),
		row('b', { type: 'paused' }, { category: 'Video' }),
		row('c', { type: 'completed' }, { kind: 'torrent' }),
		row('d', { type: 'failed', message: 'x', retryable: true })
	];

	it('match the sidebar entries', () => {
		const ids = (f: Parameters<typeof matchesFilter>[1]) =>
			rows.filter((r) => matchesFilter(r, f)).map((r) => r.id);
		expect(ids({ type: 'active' })).toEqual(['a']);
		expect(ids({ type: 'queued' })).toEqual(['b']);
		expect(ids({ type: 'completed' })).toEqual(['c']);
		expect(ids({ type: 'failed' })).toEqual(['d']);
		expect(ids({ type: 'torrents' })).toEqual(['c']);
		expect(ids({ type: 'category', name: 'Video' })).toEqual(['b']);
	});

	it('combine with search and count per entry', () => {
		expect(applyFilters(rows, { type: 'all' }, '  B.BIN ').map((r) => r.id)).toEqual(['b']);
		expect(applyFilters(rows, { type: 'all' }, '')).toBe(rows);
		const counts = countByFilter(rows, ['Video', 'Music']);
		expect(counts.get('all')).toBe(4);
		expect(counts.get('category:Video')).toBe(1);
		expect(counts.get('category:Music')).toBe(0);
	});
});

describe('selection', () => {
	const order = ['a', 'b', 'c', 'd', 'e'];
	const plain = { ctrl: false, shift: false };

	it('click, ctrl+click and shift+click like a file manager', () => {
		let s = click(EMPTY, 'b', order, plain);
		expect([...s.ids]).toEqual(['b']);
		s = click(s, 'd', order, { ctrl: false, shift: true });
		expect([...s.ids]).toEqual(['b', 'c', 'd']);
		s = click(s, 'c', order, { ctrl: true, shift: false });
		expect([...s.ids].sort()).toEqual(['b', 'd']);
		expect(s.anchor).toBe('c');
		expect([...selectAll(order).ids]).toEqual(order);
	});

	it('arrows move the focus and shift extends from the anchor', () => {
		let s = move(EMPTY, order, 1, false);
		expect(s.focus).toBe('a');
		s = move(s, order, 1, true);
		s = move(s, order, 1, true);
		expect([...s.ids]).toEqual(['a', 'b', 'c']);
		expect(move(s, order, 99, false).focus).toBe('e');
		expect(move(EMPTY, order, -1, false).focus).toBe('e');
		expect(move(EMPTY, [], 1, false)).toEqual(EMPTY);
	});

	it('prune drops rows that disappeared and keeps the object otherwise', () => {
		const s = click(selectAll(order), 'b', order, { ctrl: true, shift: false });
		expect(prune(s, order)).toBe(s);
		const pruned = prune(s, ['a', 'c']);
		expect([...pruned.ids]).toEqual(['a', 'c']);
		expect(pruned.focus).toBeNull();
	});
});

describe('speed history', () => {
	it('keeps the last N samples per task', () => {
		const h = new Map<string, number[]>();
		for (let i = 1; i <= 5; i++) {
			recordSpeeds(
				h,
				[
					{
						id: 'a',
						downloaded: 0,
						total: null,
						speedBps: i,
						etaSecs: null,
						connections: 1,
						peers: null,
						uploadBps: null,
						checkPercent: null
					}
				],
				3
			);
		}
		expect(h.get('a')).toEqual([3, 4, 5]);
	});

	it('scales the sparkline to the box', () => {
		expect(sparklinePath([0, 50, 100], 100, 20)).toBe('M0 20 L50 10 L100 0');
		expect(sparklinePath([], 100, 20)).toBe('');
		expect(sparklinePath([0, 0], 10, 10)).toBe('M0 10 L10 10');
	});
});

describe('shortcuts', () => {
	const key = (k: string, mods: Partial<KeyboardEvent> = {}) => ({
		key: k,
		ctrlKey: false,
		metaKey: false,
		shiftKey: false,
		altKey: false,
		...mods
	});

	it('map keys to actions outside text fields only', () => {
		expect(actionFor(key(' '), false)).toBe('toggle-pause');
		expect(actionFor(key('Delete'), false)).toBe('remove');
		expect(actionFor(key('a', { ctrlKey: true }), false)).toBe('select-all');
		expect(actionFor(key('A', { metaKey: true }), false)).toBe('select-all');
		expect(actionFor(key('ArrowDown', { shiftKey: true }), false)).toBe('extend-down');
		expect(actionFor(key(' '), true)).toBeNull();
		expect(actionFor(key('a', { ctrlKey: true }), true)).toBeNull();
		expect(actionFor(key('Escape'), true)).toBe('escape');
		expect(actionFor(key('x'), false)).toBeNull();
	});
});
