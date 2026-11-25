<!--
	A download from the browser, before it starts: what it is, where it comes from and where
	it goes. Closing it means no: the browser's own download is already cancelled.
-->
<script lang="ts">
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { FileDown, FolderOpen, KeyRound, TriangleAlert } from '@lucide/svelte';
	import { commands, errorMessage, type OfferView } from '#lib/api/index.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';
	import { describeType, isProgram } from '#lib/utils/filetype.ts';
	import { formatBytes } from '#lib/utils/format.ts';
	import Modal from './Modal.svelte';

	type Props = {
		offer: OfferView | null;
		/** How many more wait after this one. */
		waiting: number;
		categories: string[];
		onanswered: (id: string) => void;
	};
	let { offer, waiting, categories, onanswered }: Props = $props();

	/** The dialog comes up by itself: a key meant for the browser must not answer it. */
	const ARM_MS = 500;

	let fileName = $state('');
	let destDir = $state('');
	let category = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);
	let armed = $state(false);

	// A new question: its own values, and a moment before it can be answered.
	let shown: string | null = null;
	$effect(() => {
		const next = offer;
		if (next === null || next.id === shown) {
			if (next === null) shown = null;
			return;
		}
		shown = next.id;
		fileName = next.fileName;
		destDir = next.destDir;
		category = '';
		error = null;
		armed = false;
		setTimeout(() => {
			if (shown === next.id) armed = true;
		}, ARM_MS);
	});

	let type = $derived(offer ? describeType(fileName || offer.fileName, offer.mime) : '');
	let program = $derived(isProgram(fileName));
	let canAccept = $derived(armed && !busy && fileName.trim() !== '' && destDir.trim() !== '');

	async function browse() {
		const dir = await openDialog({ directory: true, defaultPath: destDir || undefined });
		if (typeof dir === 'string') destDir = dir;
	}

	async function accept(e: SubmitEvent) {
		e.preventDefault();
		if (!offer || !canAccept) return;
		busy = true;
		error = null;
		const { id, fileName: suggested } = offer;
		try {
			await commands.acceptOffer(id, {
				destDir: destDir.trim(),
				// Unchanged: the server may still name the file better.
				fileName: fileName.trim() === suggested ? null : fileName.trim(),
				category: category || null
			});
			toasts.info('Download added');
			onanswered(id);
		} catch (err) {
			error = errorMessage(err);
		} finally {
			busy = false;
		}
	}

	async function decline() {
		if (!offer || busy) return;
		const { id } = offer;
		try {
			await commands.declineOffer(id);
		} catch (err) {
			toasts.error(errorMessage(err));
		}
		onanswered(id);
	}
</script>

<!-- A dialog per question: one closed with Escape would not open for the next. -->
{#key offer?.id}
<Modal open={offer !== null} title="Download this file?" onclose={decline} sticky>
	{#if offer}
		<form id="offer-form" class="space-y-4" onsubmit={accept}>
			<div class="flex items-center gap-4">
				<span
					class="grid size-14 shrink-0 place-items-center rounded-2xl bg-(--control) text-teal-700 dark:text-teal-300"
					aria-hidden="true"
				>
					<FileDown size={26} />
				</span>
				<div class="min-w-0">
					<p class="text-base font-semibold break-words">{offer.fileName}</p>
					<p class="text-sm text-(--muted)">
						{offer.size === null ? 'Size unknown' : formatBytes(offer.size)} · {type}
					</p>
				</div>
			</div>

			<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
				<dt class="text-(--muted)">From</dt>
				<dd class="min-w-0 font-medium break-words">{offer.host}</dd>
				{#if offer.pageHost}
					<dt class="text-(--muted)">Started on</dt>
					<dd class="min-w-0 break-words">{offer.pageHost}</dd>
				{/if}
				<dt class="text-(--muted)">Link</dt>
				<dd class="max-h-16 min-w-0 overflow-y-auto text-xs leading-5 break-all select-text">{offer.url}</dd>
			</dl>

			{#if offer.sendsCredentials}
				<p class="flex items-start gap-2 text-xs text-(--muted)">
					<KeyRound size={14} class="mt-0.5 shrink-0" aria-hidden="true" />
					Your cookies for {offer.host} go along, so files behind a login work. They are sent only to that site.
				</p>
			{/if}
			{#if program}
				<p class="flex items-start gap-2 rounded-xl bg-amber-500/15 p-2.5 text-xs text-amber-900 dark:text-amber-100">
					<TriangleAlert size={14} class="mt-0.5 shrink-0" aria-hidden="true" />
					This is a program. Download it only if you trust {offer.host}. Persivio never runs it.
				</p>
			{/if}

			<div>
				<label class="label" for="offer-name">Save as</label>
				<input id="offer-name" class="input" maxlength="200" spellcheck="false" bind:value={fileName} />
			</div>
			<div class="grid gap-3 sm:grid-cols-[1fr_11rem]">
				<div>
					<label class="label" for="offer-dir">In folder</label>
					<div class="flex gap-2">
						<input id="offer-dir" class="input" spellcheck="false" bind:value={destDir} />
						<button type="button" class="btn shrink-0" onclick={browse}><FolderOpen size={14} /> Browse</button>
					</div>
				</div>
				<div>
					<label class="label" for="offer-category">Category</label>
					<select id="offer-category" class="input" bind:value={category}>
						<option value="">None</option>
						{#each categories as c (c)}<option value={c}>{c}</option>{/each}
					</select>
				</div>
			</div>
			{#if error}<p class="field-error" role="alert">{error}</p>{/if}
		</form>
	{/if}
	{#snippet footer()}
		{#if waiting > 0}
			<span class="mr-auto self-center text-xs text-(--muted)">{waiting} more waiting</span>
		{/if}
		<button class="btn" type="button" disabled={busy} onclick={decline}>Cancel</button>
		<button class="btn-primary" type="submit" form="offer-form" disabled={!canAccept}>
			{busy ? 'Adding…' : 'Download'}
		</button>
	{/snippet}
</Modal>
{/key}
