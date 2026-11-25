// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const w = vi.hoisted(() => ({
	decorated: false,
	maximized: false,
	resized: () => {},
	minimize: vi.fn(),
	toggleMaximize: vi.fn(),
	close: vi.fn()
}));
vi.mock('@tauri-apps/api/window', () => ({
	getCurrentWindow: () => ({
		isDecorated: async () => w.decorated,
		isMaximized: async () => w.maximized,
		onResized: async (f: () => void) => {
			w.resized = f;
		},
		minimize: w.minimize,
		toggleMaximize: w.toggleMaximize,
		close: w.close
	})
}));

const { appWindow } = await import('#lib/stores/window.svelte.ts');
const { default: WindowControls } = await import('./WindowControls.svelte');

beforeEach(() => {
	w.decorated = false;
	w.maximized = false;
	appWindow.ownControls = false;
	appWindow.maximized = false;
});

describe('WindowControls', () => {
	it('is absent when the system draws the title bar', async () => {
		w.decorated = true;
		await appWindow.init();
		render(WindowControls);
		expect(screen.queryByRole('group')).toBeNull();
	});

	it('minimizes, maximizes and closes the window', async () => {
		await appWindow.init();
		render(WindowControls);
		await fireEvent.click(screen.getByRole('button', { name: 'Minimize' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Maximize' }));
		await fireEvent.click(screen.getByRole('button', { name: 'Close window' }));
		expect(w.minimize).toHaveBeenCalledOnce();
		expect(w.toggleMaximize).toHaveBeenCalledOnce();
		expect(w.close).toHaveBeenCalledOnce();
	});

	it('offers to restore a window that was maximized in any way', async () => {
		await appWindow.init();
		render(WindowControls);
		w.maximized = true;
		w.resized();
		await vi.waitFor(async () => {
			await tick();
			expect(screen.getByRole('button', { name: 'Restore' })).toBeTruthy();
		});
	});
});

describe('Modal', () => {
	it('does not put the keyboard on the window buttons', async () => {
		// jsdom has no showModal; a browser focuses the first control.
		HTMLDialogElement.prototype.showModal = function () {
			this.open = true;
			this.querySelector('button')?.focus();
		};
		await appWindow.init();
		const { createRawSnippet } = await import('svelte');
		const { default: Modal } = await import('./Modal.svelte');
		render(Modal, {
			open: true,
			title: 'News',
			onclose: () => {},
			children: createRawSnippet(() => ({ render: () => '<p>text</p>' }))
		});
		await tick();
		expect(screen.getByRole('button', { name: 'Minimize' })).toBeTruthy();
		expect(document.activeElement?.hasAttribute('data-panel')).toBe(true);
	});
});
