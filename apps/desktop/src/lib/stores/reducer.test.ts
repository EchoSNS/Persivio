import { describe, expect, it } from 'vitest';
import type { ProgressView, TaskView } from '#lib/api/bindings.ts';
import { applyEvent, applyProgress, fromList, isActive } from './reducer.ts';

function task(id: string, overrides: Partial<TaskView> = {}): TaskView {
	return {
		id,
		kind: 'http',
		name: `${id}.bin`,
		source: `https://example.com/${id}.bin`,
		destDir: '/downloads',
		state: { type: 'queued' },
		total: null,
		downloaded: 0,
		createdAt: 1,
		completedAt: null,
		category: null,
		...overrides
	};
}

function progress(id: string, downloaded: number, speedBps = 100): ProgressView {
	return {
		id,
		downloaded,
		total: 1000,
		speedBps,
		etaSecs: 9,
		connections: 4,
		peers: null,
		uploadBps: null,
		checkPercent: null
	};
}

describe('checking', () => {
	it('is active, reports its own percentage, and forgets it afterwards', () => {
		let rows = fromList([task('a', { downloaded: 400 })]);
		rows = applyEvent(rows, { type: 'state_changed', id: 'a', state: { type: 'checking' } }).rows;
		expect(isActive(rows[0]!.state)).toBe(true);
		rows = applyProgress(rows, [{ ...progress('a', 400, 0), checkPercent: 37 }]);
		expect([rows[0]!.checkPercent, rows[0]!.downloaded]).toEqual([37, 400]);
		rows = applyEvent(rows, { type: 'state_changed', id: 'a', state: { type: 'downloading' } }).rows;
		expect(rows[0]!.checkPercent).toBeNull();
	});
});

describe('applyEvent', () => {
	it('adds a task once, even if it is already in the list', () => {
		const rows = fromList([task('a')]);
		const added = applyEvent(rows, { type: 'task_added', task: task('b') }).rows;
		expect(added.map((r) => r.id)).toEqual(['a', 'b']);
		const again = applyEvent(added, { type: 'task_added', task: task('b') }).rows;
		expect(again).toBe(added);
	});

	it('clears live numbers when a task stops and fills the bar on completion', () => {
		let rows = fromList([task('a', { total: 1000, state: { type: 'downloading' } })]);
		rows = applyProgress(rows, [progress('a', 600)]);
		expect(rows[0]?.speedBps).toBe(100);

		const paused = applyEvent(rows, { type: 'state_changed', id: 'a', state: { type: 'paused' } }).rows;
		expect(paused[0]).toMatchObject({ speedBps: 0, etaSecs: null, connections: 0, downloaded: 600 });

		const done = applyEvent(rows, { type: 'state_changed', id: 'a', state: { type: 'completed' } }).rows;
		expect(done[0]?.downloaded).toBe(1000);
		expect(done[0]?.completedAt).not.toBeNull();
	});

	it('keeps the failure message', () => {
		const rows = fromList([task('a')]);
		const failed = applyEvent(rows, {
			type: 'state_changed',
			id: 'a',
			state: { type: 'failed', message: 'download link expired', retryable: false }
		}).rows;
		expect(failed[0]?.state).toEqual({ type: 'failed', message: 'download link expired', retryable: false });
	});

	it('applies metadata, removal, warnings and resync requests', () => {
		const rows = fromList([task('a'), task('b')]);
		const named = applyEvent(rows, {
			type: 'metadata_resolved',
			id: 'a',
			name: 'ubuntu.iso',
			size: 5000,
			files: []
		}).rows;
		expect(named[0]).toMatchObject({ name: 'ubuntu.iso', total: 5000 });

		expect(applyEvent(named, { type: 'task_removed', id: 'a' }).rows.map((r) => r.id)).toEqual(['b']);
		expect(applyEvent(rows, { type: 'warning', id: null, message: 'disk almost full' }).warning).toBe(
			'disk almost full'
		);
		expect(applyEvent(rows, { type: 'resync' }).resync).toBe(true);
	});

	it('ignores events for unknown tasks', () => {
		const rows = fromList([task('a')]);
		expect(applyEvent(rows, { type: 'state_changed', id: 'zzz', state: { type: 'paused' } }).rows).toBe(rows);
	});
});

describe('applyProgress', () => {
	it('updates only the tasks in the batch and returns the same array for no-ops', () => {
		const rows = fromList([task('a'), task('b')]);
		const next = applyProgress(rows, [progress('b', 300, 2048)]);
		expect(next[0]).toBe(rows[0]);
		expect(next[1]).toMatchObject({ downloaded: 300, speedBps: 2048, total: 1000, connections: 4 });
		expect(applyProgress(rows, [])).toBe(rows);
		expect(applyProgress(rows, [progress('unknown', 1)])).toBe(rows);
	});
});

describe('isActive', () => {
	it('matches the engine running states', () => {
		expect(isActive({ type: 'downloading' })).toBe(true);
		expect(isActive({ type: 'seeding' })).toBe(true);
		expect(isActive({ type: 'paused' })).toBe(false);
		expect(isActive({ type: 'queued' })).toBe(false);
	});
});
