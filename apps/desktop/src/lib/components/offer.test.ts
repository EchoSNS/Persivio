// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { OfferView } from '#lib/api/bindings.ts';
import { describeType, isProgram } from '#lib/utils/filetype.ts';

const api = vi.hoisted(() => ({ acceptOffer: vi.fn(), declineOffer: vi.fn() }));
vi.mock('#lib/api/index.ts', () => ({ commands: api, errorMessage: (e: unknown) => String(e) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

const { default: OfferDialog } = await import('./OfferDialog.svelte');

const offer = (extra: Partial<OfferView> = {}): OfferView => ({
	id: '1',
	fileName: 'photos.zip',
	url: 'https://cdn.example/photos.zip',
	host: 'cdn.example',
	pageHost: 'shop.example',
	size: 5 * 1048576,
	mime: 'application/zip',
	sendsCredentials: true,
	destDir: 'D:\\Downloads',
	...extra
});

beforeEach(() => {
	vi.useRealTimers();
	api.acceptOffer.mockReset().mockResolvedValue('task');
	api.declineOffer.mockReset().mockResolvedValue(null);
	HTMLDialogElement.prototype.showModal = function () {
		this.open = true;
	};
	HTMLDialogElement.prototype.close = function () {
		this.open = false;
	};
});

const show = (o: OfferView | null = offer(), waiting = 0) => {
	const onanswered = vi.fn();
	render(OfferDialog, { offer: o, waiting, categories: ['Photos'], onanswered });
	return onanswered;
};
const button = () => screen.getByRole<HTMLButtonElement>('button', { name: 'Download', hidden: true });
const armed = () => vi.waitFor(() => expect(button().disabled).toBe(false), { timeout: 2000 });

describe('OfferDialog', () => {
	it('shows what the file is, where it comes from and where it goes', () => {
		show(offer(), 2);
		const text = document.body.textContent ?? '';
		for (const part of ['photos.zip', '5.0 MiB', 'Compressed archive (.zip)', 'cdn.example', 'shop.example', 'cookies', '2 more waiting']) {
			expect(text).toContain(part);
		}
		expect(screen.getByLabelText<HTMLInputElement>('In folder').value).toBe('D:\\Downloads');
		expect(text).not.toContain('This is a program');
	});

	it('warns about programs, also after renaming', async () => {
		show(offer({ size: null }));
		expect(document.body.textContent).toContain('Size unknown');
		await fireEvent.input(screen.getByLabelText('Save as'), { target: { value: 'setup.EXE' } });
		expect(document.body.textContent).toContain('This is a program');
	});

	it('cannot be answered by a key meant for the browser', async () => {
		const onanswered = show();
		await fireEvent.submit(document.querySelector('form')!);
		expect(api.acceptOffer).not.toHaveBeenCalled();
		expect(onanswered).not.toHaveBeenCalled();
		await armed();
	});

	it('downloads with the chosen name and folder', async () => {
		const onanswered = show();
		await armed();
		await fireEvent.click(button());
		await vi.waitFor(() => expect(onanswered).toHaveBeenCalledWith('1'));
		expect(api.acceptOffer).toHaveBeenCalledWith('1', { destDir: 'D:\\Downloads', fileName: null, category: null });
	});

	it('keeps the question and says why when the download cannot be added', async () => {
		api.acceptOffer.mockRejectedValue('folder must be absolute');
		const onanswered = show();
		await armed();
		await fireEvent.click(button());
		await vi.waitFor(() => expect(screen.getByRole('alert', { hidden: true }).textContent).toContain('folder must be absolute'));
		expect(onanswered).not.toHaveBeenCalled();
	});

	it('can be answered after the list was loaded again', async () => {
		const props = { offer: offer(), waiting: 0, categories: [], onanswered: vi.fn() };
		const { rerender } = render(OfferDialog, props);
		// The same link was clicked again at once.
		await rerender({ offer: offer() });
		await armed();
	});

	it('opens again for the next question after Escape', async () => {
		const { rerender } = render(OfferDialog, { offer: offer(), waiting: 1, categories: [], onanswered: vi.fn() });
		const dialog = document.querySelector('dialog')!;
		dialog.open = false; // what Escape does
		await fireEvent(dialog, new Event('close'));
		await rerender({ offer: offer({ id: '2', fileName: 'second.zip' }), waiting: 0 });
		expect(document.querySelector('dialog')!.open).toBe(true);
		expect(screen.getByLabelText<HTMLInputElement>('Save as').value).toBe('second.zip');
	});

	it('is not answered by a click next to it', async () => {
		const onanswered = show();
		await fireEvent.click(document.querySelector('dialog')!);
		expect(onanswered).not.toHaveBeenCalled();
		await fireEvent.click(screen.getByRole('button', { name: 'Cancel', hidden: true }));
		expect(api.declineOffer).toHaveBeenCalledWith('1');
		await vi.waitFor(() => expect(onanswered).toHaveBeenCalledWith('1'));
	});
});

describe('file types', () => {
	it('are named for people', () => {
		expect(describeType('a.tar.GZ', null)).toBe('Compressed archive (.gz)');
		expect(describeType('a.xyz', 'application/x')).toBe('.xyz file');
		expect(describeType('a', 'image/heic')).toBe('image/heic');
		expect(describeType('a', 'application/octet-stream')).toBe('Unknown type');
	});

	it('recognize programs in disguise', () => {
		for (const name of ['invoice.pdf.exe', 'a.EXE', 'a.exe. .', 'x.scr', 'x.ps1']) expect(isProgram(name), name).toBe(true);
		for (const name of ['a.pdf', 'exe', '.exe', 'a.zip']) expect(isProgram(name), name).toBe(false);
	});
});
