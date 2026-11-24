// @vitest-environment jsdom
import { X } from '@lucide/svelte';
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { TaskView } from '#lib/api/bindings.ts';
import { toRow, type TaskRow as Row } from '#lib/stores/reducer.ts';
import { EMPTY } from '#lib/stores/selection.ts';
import ContextMenu from './ContextMenu.svelte';
import FileSelector from './FileSelector.svelte';
import SegmentBar from './SegmentBar.svelte';
import StateBadge from './StateBadge.svelte';
import TaskList from './TaskList.svelte';
import TaskRow from './TaskRow.svelte';

function row(id: string, extra: Partial<TaskView> = {}, live: Partial<Row> = {}): Row {
	return {
		...toRow({
			id,
			kind: 'http',
			name: `${id}.iso`,
			source: `https://example.com/${id}.iso`,
			destDir: '/d',
			state: { type: 'downloading' },
			total: 1000,
			downloaded: 250,
			createdAt: 0,
			completedAt: null,
			category: null,
			...extra
		}),
		...live
	};
}

function handlers() {
	return {
		onselect: vi.fn(),
		onopen: vi.fn(),
		onmenu: vi.fn(),
		onpause: vi.fn(),
		onresume: vi.fn(),
		onremove: vi.fn(),
		onreveal: vi.fn()
	};
}

describe('TaskRow', () => {
	it('shows name, state, progress and speed', () => {
		render(TaskRow, {
			task: row('a', {}, { speedBps: 2 * 1048576 }),
			selected: true,
			focused: false,
			...handlers()
		});
		expect(screen.getByText('a.iso')).toBeTruthy();
		expect(screen.getByText('Downloading')).toBeTruthy();
		expect(screen.getByText('25%')).toBeTruthy();
		expect(screen.getByText('2.0 MiB/s')).toBeTruthy();
		expect(screen.getByRole('option').getAttribute('aria-selected')).toBe('true');
	});

	it('row buttons act without changing the selection', async () => {
		const h = handlers();
		render(TaskRow, { task: row('a'), selected: false, focused: false, ...h });
		await fireEvent.click(screen.getByRole('button', { name: 'Pause a.iso' }));
		expect(h.onpause).toHaveBeenCalledOnce();
		expect(h.onselect).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('option'));
		expect(h.onselect).toHaveBeenCalledOnce();
		await fireEvent.dblClick(screen.getByRole('option'));
		expect(h.onopen).toHaveBeenCalledOnce();
	});

	it('offers retry only for retryable failures and "show" for finished files', () => {
		const h = handlers();
		const { unmount } = render(TaskRow, {
			task: row('a', { state: { type: 'failed', message: 'HTTP 503', retryable: true } }),
			selected: false,
			focused: false,
			...h
		});
		expect(screen.getByRole('button', { name: 'Retry a.iso' })).toBeTruthy();
		unmount();
		render(TaskRow, {
			task: row('b', { state: { type: 'completed' }, downloaded: 1000 }),
			selected: false,
			focused: false,
			...h
		});
		expect(screen.getByRole('button', { name: 'Show b.iso in folder' })).toBeTruthy();
		expect(screen.queryByRole('button', { name: /Pause|Resume|Retry/ })).toBeNull();
	});
});

describe('checking existing data', () => {
	it('shows the check, not a download: own percentage, no speed', () => {
		render(TaskRow, {
			task: row('a', { state: { type: 'checking' } }, { checkPercent: 80, speedBps: 999_999 }),
			selected: false,
			focused: false,
			...handlers()
		});
		expect(screen.getByText('Checking files')).toBeTruthy();
		expect(screen.getByText('80%')).toBeTruthy();
		expect(screen.queryByText(/[/]s$/)).toBeNull();
		expect(
			screen.getByRole('progressbar', { name: 'Check progress of a.iso' }).getAttribute('aria-valuenow')
		).toBe('80');
	});
});

describe('ContextMenu', () => {
	it('moves with the arrow keys, skips disabled items, runs and closes', async () => {
		const run = vi.fn();
		const onclose = vi.fn();
		const outside = vi.fn();
		document.body.addEventListener('keydown', outside);
		render(ContextMenu, {
			x: 5,
			y: 5,
			label: 'Actions',
			onclose,
			items: [
				{ label: 'Pause', icon: X, run: vi.fn() },
				{ label: 'Nope', icon: X, run: vi.fn(), disabled: true },
				'separator',
				{ label: 'Remove', icon: X, run }
			]
		});
		const pause = screen.getByRole('menuitem', { name: 'Pause' });
		const remove = screen.getByRole('menuitem', { name: 'Remove' });
		expect(document.activeElement).toBe(pause);
		await fireEvent.keyDown(pause, { key: 'ArrowDown' });
		expect(document.activeElement).toBe(remove);
		await fireEvent.keyDown(remove, { key: 'ArrowDown' });
		expect(document.activeElement).toBe(pause);
		await fireEvent.click(remove);
		expect(run).toHaveBeenCalledOnce();
		expect(onclose).toHaveBeenCalledOnce();
		await fireEvent.keyDown(remove, { key: 'Escape' });
		expect(onclose).toHaveBeenCalledTimes(2);
		expect(outside).not.toHaveBeenCalled();
		document.body.removeEventListener('keydown', outside);
	});
});

describe('TaskList', () => {
	it('renders only a window of 500 rows (virtualized)', () => {
		const rows = Array.from({ length: 500 }, (_, i) => row(`t${i}`));
		const { container } = render(TaskList, {
			rows,
			selection: EMPTY,
			label: 'All downloads',
			...handlers()
		});
		const rendered = screen.queryAllByRole('option').length;
		expect(rendered).toBeLessThan(40);
		const spacer = container.querySelector('[role="listbox"] > div') as HTMLElement;
		expect(spacer.style.height).toBe(`${500 * 74}px`);
	});
});

describe('SegmentBar', () => {
	it('draws each segment at its byte position, filled by its progress', () => {
		const { container } = render(SegmentBar, {
			total: 1000,
			segments: [
				{ start: 0, end: 499, downloaded: 500 },
				{ start: 500, end: 999, downloaded: 125 }
			]
		});
		const rects = [...container.querySelectorAll('[data-testid="segment"]')];
		expect(rects.map((r) => [r.getAttribute('x'), r.getAttribute('width')])).toEqual([
			['0', '500'],
			['500', '125']
		]);
	});
});

describe('FileSelector', () => {
	it('toggles files and reports the chosen size', async () => {
		const onchange = vi.fn();
		render(FileSelector, {
			files: [
				{ index: 0, path: 'a.bin', size: 1024, selected: true },
				{ index: 1, path: 'b.bin', size: 2048, selected: true }
			],
			selected: new Set([0, 1]),
			onchange
		});
		expect(screen.getByText(/2 of 2 files · 3.0 KiB/)).toBeTruthy();
		await fireEvent.click(screen.getByRole('checkbox', { name: /b\.bin/ }));
		expect(onchange).toHaveBeenCalledWith(new Set([0]));
	});
});

describe('StateBadge', () => {
	it('explains failures on hover', () => {
		render(StateBadge, { state: { type: 'failed', message: 'download link expired', retryable: false } });
		expect(screen.getByText('Failed').getAttribute('title')).toBe('download link expired');
	});
});
