<script lang="ts" module>
	export type TorrentDraft = { name: string; data: string };
</script>

<script lang="ts">
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { FileUp, FolderOpen, Globe, Magnet, TriangleAlert } from '@lucide/svelte';
	import {
		commands,
		errorMessage,
		type AddRequest,
		type ChecksumAlgorithmView,
		type TorrentInfoView
	} from '#lib/api/index.ts';
	import type { TaskStore } from '#lib/stores/tasks.svelte.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';
	import { formatBytes } from '#lib/utils/format.ts';
	import {
		classifyLink,
		fileToBase64,
		isTorrentFile,
		parseHeaders,
		splitLinks
	} from '#lib/utils/sources.ts';
	import FileSelector from './FileSelector.svelte';
	import Modal from './Modal.svelte';

	type Props = {
		open: boolean;
		store: TaskStore;
		categories: string[];
		initialLinks: string[];
		initialTorrent: TorrentDraft | null;
		onclose: () => void;
	};
	let { open, store, categories, initialLinks, initialTorrent, onclose }: Props = $props();

	let linksText = $state('');
	let torrent = $state<(TorrentDraft & { info: TorrentInfoView | null }) | null>(null);
	let torrentSelection = $state(new Set<number>());
	let destDir = $state('');
	let category = $state('');
	let connections = $state('');
	let limit = $state('');
	let limitUnit = $state<'KiB' | 'MiB'>('MiB');
	let headersText = $state('');
	let cookies = $state('');
	let referer = $state('');
	let checksumAlgorithm = $state<ChecksumAlgorithmView>('sha256');
	let checksumHex = $state('');
	let busy = $state(false);
	let torrentError = $state<string | null>(null);

	let links = $derived(splitLinks(linksText).map((l) => ({ link: l, kind: classifyLink(l) })));
	let valid = $derived(links.filter((l) => l.kind !== 'invalid'));
	let invalidCount = $derived(links.length - valid.length);
	let headers = $derived(parseHeaders(headersText));
	let hasHttp = $derived(valid.some((l) => l.kind === 'http'));
	let connectionsValue = $derived(connections.trim() === '' ? null : Number(connections));
	let connectionsError = $derived(
		connectionsValue !== null &&
			(!Number.isInteger(connectionsValue) || connectionsValue < 1 || connectionsValue > 32)
	);
	let limitValue = $derived(limit.trim() === '' ? null : Number(limit));
	let limitError = $derived(limitValue !== null && !(limitValue > 0));
	let canSubmit = $derived(
		!busy &&
			(valid.length > 0 || (torrent?.info != null && torrentSelection.size > 0)) &&
			!connectionsError &&
			!limitError &&
			headers.invalidLines.length === 0
	);

	// Reset the form each time the dialog opens with new content.
	$effect(() => {
		if (!open) return;
		linksText = initialLinks.join('\n');
		torrent = null;
		torrentError = null;
		if (initialTorrent) void loadTorrent(initialTorrent);
		if (destDir === '') {
			commands.defaultDownloadDir().then(
				(d) => (destDir ||= d),
				() => {}
			);
		}
	});

	async function loadTorrent(draft: TorrentDraft) {
		torrent = { ...draft, info: null };
		torrentError = null;
		try {
			const info = await commands.inspectTorrent(draft.data);
			torrent = { ...draft, info };
			torrentSelection = new Set(info.files.map((f) => f.index));
		} catch (e) {
			torrent = null;
			torrentError = `${draft.name}: ${errorMessage(e)}`;
		}
	}

	async function pickTorrent(e: Event & { currentTarget: HTMLInputElement }) {
		const file = e.currentTarget.files?.[0];
		e.currentTarget.value = '';
		if (!file) return;
		if (!isTorrentFile(file)) {
			torrentError = `${file.name} is not a .torrent file`;
			return;
		}
		try {
			await loadTorrent({ name: file.name, data: await fileToBase64(file) });
		} catch (err) {
			torrentError = errorMessage(err);
		}
	}

	async function browse() {
		const dir = await openDialog({ directory: true, defaultPath: destDir || undefined });
		if (typeof dir === 'string') destDir = dir;
	}

	function base(): Omit<AddRequest, 'source'> {
		return {
			destDir: destDir.trim() || null,
			fileName: null,
			connections: connectionsValue,
			speedLimitBps:
				limitValue === null ? null : Math.round(limitValue * (limitUnit === 'MiB' ? 1048576 : 1024)),
			category: category || null,
			headers: [],
			cookies: null,
			referer: null,
			checksum: null,
			selectedFiles: null
		};
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (!canSubmit) return;
		busy = true;
		const failed: string[] = [];
		let added = 0;
		const singleHttp = valid.length === 1 && valid[0]?.kind === 'http';
		for (const { link, kind } of valid) {
			const http = kind === 'http';
			const id = await store.add({
				...base(),
				source: { type: 'link', url: link },
				headers: http ? headers.headers : [],
				cookies: http ? cookies.trim() || null : null,
				referer: http ? referer.trim() || null : null,
				// A checksum only makes sense for a single file.
				checksum:
					singleHttp && checksumHex.trim()
						? { algorithm: checksumAlgorithm, hex: checksumHex.trim() }
						: null
			});
			if (id) added++;
			else failed.push(link);
		}
		if (torrent?.info) {
			const all = torrentSelection.size === torrent.info.files.length;
			const id = await store.add({
				...base(),
				source: { type: 'torrent_file', data: torrent.data },
				selectedFiles: all ? null : [...torrentSelection].sort((a, b) => a - b)
			});
			if (id) {
				added++;
				torrent = null;
			}
		}
		busy = false;
		linksText = failed.join('\n');
		if (added > 0) toasts.info(added === 1 ? 'Download added' : `${added} downloads added`);
		if (failed.length === 0 && torrent === null) onclose();
	}
</script>

<Modal {open} title="Add downloads" {onclose} wide>
	<form id="add-form" class="space-y-4" onsubmit={submit}>
		<div>
			<label class="label" for="add-links">Links (one per line)</label>
			<textarea
				id="add-links"
				data-autofocus
				class="input min-h-24 font-mono text-xs"
				placeholder="https://example.com/file.iso&#10;magnet:?xt=urn:btih:..."
				spellcheck="false"
				bind:value={linksText}
			></textarea>
			{#if links.length > 0}
				<ul class="mt-2 flex flex-wrap gap-1.5" aria-label="Detected links">
					{#each links.slice(0, 50) as l (l.link)}
						<li
							class="inline-flex max-w-full items-center gap-1 rounded-full px-2 py-0.5 text-[11px]
								{l.kind === 'invalid'
								? 'bg-red-500/15 text-red-800 dark:text-red-200'
								: 'border border-(--line) bg-(--control)'}"
							title={l.link}
						>
							{#if l.kind === 'magnet'}<Magnet size={11} />{:else if l.kind === 'http'}<Globe size={11} />{:else}<TriangleAlert size={11} />{/if}
							<span class="max-w-60 truncate">{l.link}</span>
						</li>
					{/each}
				</ul>
				{#if invalidCount > 0}
					<p class="field-error">
						{invalidCount} line{invalidCount === 1 ? ' is' : 's are'} not an http(s) or magnet link and will be skipped.
					</p>
				{/if}
			{/if}
		</div>

		<div>
			<span class="label">Torrent file</span>
			{#if torrent?.info}
				<div class="space-y-2 rounded-2xl bg-(--control) p-3">
					<div class="flex items-center justify-between gap-2 text-sm">
						<span class="truncate font-medium">{torrent.info.name}</span>
						<span class="text-xs text-(--muted)">{formatBytes(torrent.info.totalSize)}</span>
					</div>
					<FileSelector
						files={torrent.info.files}
						selected={torrentSelection}
						onchange={(s) => (torrentSelection = s)}
					/>
					<button type="button" class="text-xs underline" onclick={() => (torrent = null)}>
						Remove torrent
					</button>
				</div>
			{:else}
				<label class="btn cursor-pointer">
					<FileUp size={14} /> Choose a .torrent file…
					<input type="file" accept=".torrent,application/x-bittorrent" class="sr-only" onchange={pickTorrent} />
				</label>
			{/if}
			{#if torrentError}<p class="field-error">{torrentError}</p>{/if}
		</div>

		<div class="grid gap-3 sm:grid-cols-[1fr_12rem]">
			<div>
				<label class="label" for="add-dir">Save to</label>
				<div class="flex gap-2">
					<input id="add-dir" class="input" bind:value={destDir} spellcheck="false" />
					<button type="button" class="btn shrink-0" onclick={browse}>
						<FolderOpen size={14} /> Browse
					</button>
				</div>
			</div>
			<div>
				<label class="label" for="add-category">Category</label>
				<select id="add-category" class="input" bind:value={category}>
					<option value="">None</option>
					{#each categories as c (c)}<option value={c}>{c}</option>{/each}
				</select>
			</div>
		</div>

		<div class="grid gap-3 sm:grid-cols-2">
			<div>
				<label class="label" for="add-connections">Connections (HTTP)</label>
				<input
					id="add-connections"
					class="input"
					inputmode="numeric"
					placeholder="Default"
					bind:value={connections}
					aria-invalid={connectionsError}
				/>
				{#if connectionsError}<p class="field-error">Between 1 and 32.</p>{/if}
			</div>
			<div>
				<label class="label" for="add-limit">Speed limit</label>
				<div class="flex gap-2">
					<input
						id="add-limit"
						class="input"
						inputmode="decimal"
						placeholder="Unlimited"
						bind:value={limit}
						aria-invalid={limitError}
					/>
					<select class="input w-24" bind:value={limitUnit} aria-label="Speed limit unit">
						<option value="KiB">KiB/s</option>
						<option value="MiB">MiB/s</option>
					</select>
				</div>
				{#if limitError}<p class="field-error">Must be greater than zero.</p>{/if}
			</div>
		</div>

		<details class="rounded-2xl border border-(--line)">
			<summary class="cursor-pointer px-3 py-2 text-xs font-medium select-none">
				Advanced (HTTP): headers, cookies, referer, checksum
			</summary>
			<div class="space-y-3 border-t border-(--line) p-3">
				{#if !hasHttp}
					<p class="hint">These options apply to http(s) links only.</p>
				{/if}
				<div>
					<label class="label" for="add-headers">Headers (Name: value, one per line)</label>
					<textarea id="add-headers" class="input min-h-16 font-mono text-xs" spellcheck="false" bind:value={headersText}></textarea>
					{#if headers.invalidLines.length > 0}
						<p class="field-error">Invalid header on line {headers.invalidLines.join(', ')}.</p>
					{/if}
				</div>
				<div>
					<label class="label" for="add-cookies">Cookies</label>
					<input id="add-cookies" class="input font-mono text-xs" placeholder="name=value; other=value" spellcheck="false" bind:value={cookies} />
					<p class="hint">Sent only to the link's own site, never to other hosts after a redirect.</p>
				</div>
				<div>
					<label class="label" for="add-referer">Referer</label>
					<input id="add-referer" class="input" placeholder="https://example.com/page" spellcheck="false" bind:value={referer} />
				</div>
				<div>
					<label class="label" for="add-checksum">Expected checksum (single link)</label>
					<div class="flex gap-2">
						<select class="input w-28" bind:value={checksumAlgorithm} aria-label="Checksum algorithm">
							<option value="sha256">SHA-256</option>
							<option value="md5">MD5</option>
						</select>
						<input id="add-checksum" class="input font-mono text-xs" spellcheck="false" bind:value={checksumHex} />
					</div>
				</div>
			</div>
		</details>
	</form>

	{#snippet footer()}
		<button class="btn" onclick={onclose}>Cancel</button>
		<button class="btn-primary" type="submit" form="add-form" disabled={!canSubmit}>
			{busy ? 'Adding…' : 'Add'}
		</button>
	{/snippet}
</Modal>
