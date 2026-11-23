<script lang="ts">
	import { onMount } from 'svelte';
	import { Download, Inbox, Pause, Play, Plus, Search, Trash2 } from '@lucide/svelte';
	import {
		commands,
		errorMessage,
		events,
		type ExternalAddView,
		type SettingsView
	} from '#lib/api/index.ts';
	import AddDialog, { type TorrentDraft } from '#lib/components/AddDialog.svelte';
	import DetailDrawer from '#lib/components/DetailDrawer.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import SettingsPage from '#lib/components/SettingsPage.svelte';
	import Sidebar from '#lib/components/Sidebar.svelte';
	import TaskList from '#lib/components/TaskList.svelte';
	import Toasts from '#lib/components/Toasts.svelte';
	import { actionFor, isTextField } from '#lib/shortcuts.ts';
	import { applyFilters, countByFilter, type Filter } from '#lib/stores/filters.ts';
	import { isActive, type TaskRow } from '#lib/stores/reducer.ts';
	import * as sel from '#lib/stores/selection.ts';
	import { TaskStore } from '#lib/stores/tasks.svelte.ts';
	import { theme } from '#lib/stores/theme.svelte.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';
	import { fileToBase64, isTorrentFile, linksFromDrop, splitLinks } from '#lib/utils/sources.ts';

	const store = new TaskStore();

	let categories = $state<string[]>([]);
	let filter = $state<Filter>({ type: 'all' });
	let search = $state('');
	let selection = $state<sel.Selection>(sel.EMPTY);
	let detailsOpen = $state(false);
	let settingsOpen = $state(false);
	let add = $state<{ open: boolean; links: string[]; torrent: TorrentDraft | null }>({
		open: false,
		links: [],
		torrent: null
	});
	let removing = $state<{ ids: string[]; deleteFiles: boolean } | null>(null);
	let dragging = $state(false);
	let list: TaskList | undefined = $state();
	let searchInput: HTMLInputElement | undefined = $state();

	let visible = $derived(applyFilters(store.tasks, filter, search));
	let order = $derived(visible.map((r) => r.id));
	let counts = $derived(countByFilter(store.tasks, categories));
	let selectedRows = $derived(visible.filter((r) => selection.ids.has(r.id)));
	let focusedTask = $derived(
		selection.focus === null ? undefined : store.tasks.find((r) => r.id === selection.focus)
	);
	let anyActive = $derived(store.tasks.some((r) => isActive(r.state) || r.state.type === 'queued'));
	let anyPaused = $derived(store.tasks.some((r) => r.state.type === 'paused'));
	let filterLabel = $derived(
		filter.type === 'category' ? filter.name : filter.type[0]?.toUpperCase() + filter.type.slice(1)
	);

	// Rows that disappear (filtered, removed) leave the selection.
	$effect(() => {
		const pruned = sel.prune(selection, order);
		if (pruned !== selection) selection = pruned;
	});

	onMount(() => {
		const stopTheme = theme.start();
		void store.connect();
		const unlisteners = [
			events.externalRequestsEvent.listen(() => void takeExternal()),
			events.clipboardLinksEvent.listen((e) => {
				const links = e.payload.links;
				const what = links.length === 1 ? (links[0] ?? '') : `${links.length} links`;
				toasts.offer(`Copied: ${what}`, { label: 'Add', run: () => openAdd(links) });
			})
		];
		// Requests that started the app (a magnet click, an opened .torrent).
		void takeExternal();
		commands.getSettings().then(
			(s) => (categories = s.categories),
			() => {}
		);
		return () => {
			stopTheme();
			for (const u of unlisteners) void u.then((stop) => stop());
			store.disconnect();
		};
	});

	function openAdd(links: string[] = [], torrent: TorrentDraft | null = null) {
		add = { open: true, links, torrent };
	}

	// External requests wait in line: one Add dialog at a time, the user confirms each.
	const externalQueue: { links: string[]; torrent: TorrentDraft | null }[] = [];

	function enqueue(view: ExternalAddView) {
		for (const error of view.errors) toasts.error(error);
		const [first, ...rest] = view.torrents;
		if (view.links.length > 0 || first) externalQueue.push({ links: view.links, torrent: first ?? null });
		for (const torrent of rest) externalQueue.push({ links: [], torrent });
	}

	function openNextExternal() {
		if (add.open) return;
		const next = externalQueue.shift();
		if (next) {
			settingsOpen = false;
			openAdd(next.links, next.torrent);
		}
	}

	async function takeExternal() {
		try {
			for (const view of await commands.takeExternalRequests()) enqueue(view);
			openNextExternal();
		} catch (e) {
			toasts.error(errorMessage(e));
		}
	}

	function canPause(r: TaskRow) {
		return isActive(r.state) || r.state.type === 'queued';
	}
	function canResume(r: TaskRow) {
		return r.state.type === 'paused' || (r.state.type === 'failed' && r.state.retryable);
	}

	/** Space: pause the selection if anything in it runs, else resume it. */
	function togglePause(rows: TaskRow[]) {
		const running = rows.filter(canPause);
		if (running.length > 0) for (const r of running) void store.pause(r.id);
		else for (const r of rows.filter(canResume)) void store.resume(r.id);
	}

	function askRemove(ids: string[]) {
		if (ids.length > 0) removing = { ids, deleteFiles: false };
	}

	async function confirmRemove() {
		if (!removing) return;
		const { ids, deleteFiles } = removing;
		removing = null;
		for (const id of ids) await store.remove(id, deleteFiles);
	}

	function onSelect(id: string, e: MouseEvent) {
		selection = sel.click(selection, id, order, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
		list?.focus();
	}

	function onKeydown(e: KeyboardEvent) {
		const modalOpen = add.open || removing !== null;
		if (modalOpen) return; // the <dialog> handles its own keys (Escape closes it)
		const action = actionFor(e, isTextField(e.target));
		if (action === null) return;
		if (settingsOpen && !['add', 'settings', 'escape'].includes(action)) return;
		e.preventDefault();
		switch (action) {
			case 'add':
				openAdd();
				break;
			case 'settings':
				settingsOpen = true;
				break;
			case 'search':
				settingsOpen = false;
				searchInput?.focus();
				break;
			case 'select-all':
				selection = sel.selectAll(order);
				break;
			case 'toggle-pause':
				togglePause(selectedRows);
				break;
			case 'remove':
				askRemove(selectedRows.map((r) => r.id));
				break;
			case 'open-details':
				if (selection.focus) detailsOpen = true;
				break;
			case 'up':
			case 'down':
			case 'extend-up':
			case 'extend-down':
				selection = sel.move(
					selection,
					order,
					action.endsWith('up') ? -1 : 1,
					action.startsWith('extend')
				);
				list?.focus();
				break;
			case 'first':
				selection = sel.move(selection, order, -order.length, false);
				break;
			case 'last':
				selection = sel.move(selection, order, order.length, false);
				break;
			case 'escape':
				if (isTextField(e.target)) (e.target as HTMLElement).blur();
				else if (settingsOpen) settingsOpen = false;
				else if (detailsOpen) detailsOpen = false;
				else selection = sel.EMPTY;
				break;
		}
	}

	// Ctrl+V anywhere outside a text field: add the pasted links. The paste event carries
	// the clipboard text, so no clipboard permission is needed.
	function onPaste(e: ClipboardEvent) {
		if (add.open || isTextField(e.target)) return;
		const links = splitLinks(e.clipboardData?.getData('text/plain') ?? '');
		if (links.length > 0) {
			e.preventDefault();
			openAdd(links);
		}
	}

	// Drag and drop: links from a browser, or .torrent files (read here, so no file path
	// ever reaches Rust).
	function onDragOver(e: DragEvent) {
		e.preventDefault();
		dragging = true;
	}
	async function onDrop(e: DragEvent) {
		e.preventDefault();
		dragging = false;
		const data = e.dataTransfer;
		if (!data) return;
		const files = [...data.files];
		const torrentFile = files.find(isTorrentFile);
		if (torrentFile) {
			try {
				openAdd([], { name: torrentFile.name, data: await fileToBase64(torrentFile) });
			} catch (err) {
				toasts.error(err instanceof Error ? err.message : String(err));
			}
			return;
		}
		if (files.length > 0) {
			toasts.warn('Only .torrent files and links can be dropped here.');
			return;
		}
		const links = linksFromDrop(data);
		if (links.length > 0) openAdd(links);
		else toasts.warn('No http(s) or magnet link found in what was dropped.');
	}
</script>

<svelte:window
	onkeydown={onKeydown}
	onpaste={onPaste}
	ondragover={onDragOver}
	ondragleave={(e) => {
		if (e.relatedTarget === null) dragging = false;
	}}
	ondrop={onDrop}
/>

<div class="flex h-full">
	<Sidebar
		{filter}
		{counts}
		{categories}
		{settingsOpen}
		onfilter={(f) => {
			filter = f;
			settingsOpen = false;
		}}
		onsettings={() => (settingsOpen = true)}
	/>

	<main class="flex min-w-0 flex-1 flex-col">
		{#if settingsOpen}
			<SettingsPage onsaved={(s: SettingsView) => (categories = s.categories)} />
		{:else}
			<header class="flex items-center gap-2 border-b border-zinc-200 px-3 py-2.5 dark:border-zinc-800">
				<button class="btn-primary" onclick={() => openAdd()} title="New download (Ctrl+N)">
					<Plus size={14} /> Add
				</button>
				<div class="relative ml-1 max-w-72 flex-1">
					<Search size={14} class="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-zinc-400" aria-hidden="true" />
					<input
						bind:this={searchInput}
						class="input py-1 pl-8 text-xs"
						type="search"
						placeholder="Search (Ctrl+F)"
						aria-label="Search downloads"
						bind:value={search}
					/>
				</div>
				<div class="ml-auto flex items-center gap-1.5">
					{#if selectedRows.length > 0}
						<span class="mr-1 text-xs text-zinc-500" aria-live="polite">{selectedRows.length} selected</span>
						<button class="btn" disabled={!selectedRows.some(canPause)} onclick={() => selectedRows.filter(canPause).forEach((r) => store.pause(r.id))}>
							<Pause size={13} /> Pause
						</button>
						<button class="btn" disabled={!selectedRows.some(canResume)} onclick={() => selectedRows.filter(canResume).forEach((r) => store.resume(r.id))}>
							<Play size={13} /> Resume
						</button>
						<button class="btn" onclick={() => askRemove(selectedRows.map((r) => r.id))}>
							<Trash2 size={13} /> Remove
						</button>
					{:else}
						<button class="btn" disabled={!anyActive} onclick={() => store.pauseAll()}><Pause size={13} /> Pause all</button>
						<button class="btn" disabled={!anyPaused} onclick={() => store.resumeAll()}><Play size={13} /> Resume all</button>
					{/if}
				</div>
			</header>

			{#if store.unavailable}
				<div role="alert" class="m-4 rounded-md border border-red-300 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200">
					<p class="font-medium">The download engine is not running.</p>
					<p>{store.unavailable}</p>
					<p class="mt-1 text-xs">Close any running <code>persivio</code> command-line session, then restart the app.</p>
				</div>
			{/if}

			<div class="flex min-h-0 flex-1">
				<div class="min-w-0 flex-1">
					{#if store.loaded && visible.length === 0 && !store.unavailable}
						<div class="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
							{#if store.tasks.length === 0}
								<span class="grid size-12 place-items-center rounded-full bg-teal-600/10 text-teal-600"><Download size={22} /></span>
								<p class="text-sm font-medium">No downloads yet</p>
								<p class="max-w-sm text-xs text-zinc-500">
									Paste a link with <span class="kbd">Ctrl+V</span>, drop links or .torrent files here, or click Add.
								</p>
								<button class="btn-primary mt-1" onclick={() => openAdd()}><Plus size={14} /> Add a download</button>
							{:else}
								<span class="text-zinc-400"><Inbox size={28} /></span>
								<p class="text-sm text-zinc-500">
									{search.trim() ? `Nothing matches “${search.trim()}”.` : `No downloads in ${filterLabel}.`}
								</p>
							{/if}
						</div>
					{:else}
						<TaskList
							bind:this={list}
							rows={visible}
							{selection}
							label="{filterLabel} downloads"
							onselect={onSelect}
							onopen={(id) => {
								selection = sel.single(id);
								detailsOpen = true;
							}}
							onpause={(id) => store.pause(id)}
							onresume={(id) => store.resume(id)}
							onremove={(id) => askRemove([id])}
							onreveal={(id) => store.reveal(id)}
						/>
					{/if}
				</div>
				{#if detailsOpen && focusedTask}
					<DetailDrawer task={focusedTask} {store} onclose={() => (detailsOpen = false)} />
				{/if}
			</div>
		{/if}
	</main>
</div>

{#if dragging}
	<div class="pointer-events-none fixed inset-3 z-40 grid place-items-center rounded-xl border-2 border-dashed border-teal-500 bg-teal-500/10 text-sm font-medium text-teal-800 dark:text-teal-200">
		Drop links or a .torrent file to add them
	</div>
{/if}

<AddDialog
	open={add.open}
	{store}
	{categories}
	initialLinks={add.links}
	initialTorrent={add.torrent}
	onclose={() => {
		add = { open: false, links: [], torrent: null };
		openNextExternal();
	}}
/>

<Modal
	open={removing !== null}
	title={removing && removing.ids.length > 1 ? `Remove ${removing.ids.length} downloads?` : 'Remove download?'}
	onclose={() => (removing = null)}
>
	{#if removing}
		<p class="text-sm text-zinc-600 dark:text-zinc-300">
			{removing.ids.length > 1 ? 'They are' : 'It is'} removed from the list. Unfinished data is always deleted.
		</p>
		<label class="mt-3 flex items-center gap-2 text-sm">
			<input type="checkbox" class="accent-red-600" bind:checked={removing.deleteFiles} />
			Also delete downloaded files from disk
		</label>
	{/if}
	{#snippet footer()}
		<button class="btn" onclick={() => (removing = null)}>Cancel</button>
		<button class="btn-danger" onclick={confirmRemove}>
			{removing?.deleteFiles ? 'Remove and delete files' : 'Remove'}
		</button>
	{/snippet}
</Modal>

<Toasts />
