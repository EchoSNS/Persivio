<script lang="ts">
	import { onMount } from 'svelte';
	import {
		ArrowDownToLine,
		ArrowUpToLine,
		Copy,
		Download,
		FolderOpen,
		Inbox,
		Info,
		Pause,
		Play,
		Plus,
		RotateCcw,
		Search,
		Trash2
	} from '@lucide/svelte';
	import {
		commands,
		errorMessage,
		events,
		type ExternalAddView,
		type ReleaseNotesView,
		type SettingsView,
		type UpdateView
	} from '#lib/api/index.ts';
	import ReleaseNotes from '#lib/components/ReleaseNotes.svelte';
	import UpdateDialog from '#lib/components/UpdateDialog.svelte';
	import { loadLastSeen, notesToShow, saveLastSeen } from '#lib/utils/versions.ts';
	import AddDialog, { type TorrentDraft } from '#lib/components/AddDialog.svelte';
	import ContextMenu, { type MenuItem } from '#lib/components/ContextMenu.svelte';
	import DetailDrawer from '#lib/components/DetailDrawer.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import SettingsPage from '#lib/components/SettingsPage.svelte';
	import Sidebar from '#lib/components/Sidebar.svelte';
	import StatusBar from '#lib/components/StatusBar.svelte';
	import TaskList from '#lib/components/TaskList.svelte';
	import Toasts from '#lib/components/Toasts.svelte';
	import { actionFor, isTextField } from '#lib/shortcuts.ts';
	import { applyFilters, countByFilter, type Filter } from '#lib/stores/filters.ts';
	import { isActive, type TaskRow } from '#lib/stores/reducer.ts';
	import { loadSort, saveSort, sortRows, SORTS, type SortKey } from '#lib/stores/sort.ts';
	import * as sel from '#lib/stores/selection.ts';
	import { TaskStore } from '#lib/stores/tasks.svelte.ts';
	import { theme } from '#lib/stores/theme.svelte.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';
	import { fileToBase64, isTorrentFile, linksFromDrop, splitLinks } from '#lib/utils/sources.ts';

	const store = new TaskStore();

	let categories = $state<string[]>([]);
	let filter = $state<Filter>({ type: 'all' });
	let search = $state('');
	let sort = $state<SortKey>(loadSort());
	let menu = $state<{ x: number; y: number } | null>(null);
	let selection = $state<sel.Selection>(sel.EMPTY);
	let detailsOpen = $state(false);
	let settingsOpen = $state(false);
	let add = $state<{ open: boolean; links: string[]; torrent: TorrentDraft | null }>({
		open: false,
		links: [],
		torrent: null
	});
	let removing = $state<{ ids: string[]; deleteFiles: boolean } | null>(null);
	let whatsNew = $state<{ releases: ReleaseNotesView[]; version: string } | null>(null);
	let update = $state<UpdateView | null>(null);

	/** After an update (or on first start): the notes not seen yet. `all`: every release. */
	async function showWhatsNew(all: boolean) {
		try {
			const [version, releases] = await Promise.all([commands.appVersion(), commands.releaseNotes()]);
			const shown = all ? releases : notesToShow(releases, version, loadLastSeen());
			if (shown.length > 0) whatsNew = { releases: shown, version };
		} catch (e) {
			toasts.error(errorMessage(e));
		}
	}
	let dragging = $state(false);
	let list: TaskList | undefined = $state();
	let searchInput: HTMLInputElement | undefined = $state();

	let visible = $derived(sortRows(applyFilters(store.tasks, filter, search), sort));
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
			events.updateAvailableEvent.listen((e) => {
				const found = e.payload;
				toasts.offer(`Persivio ${found.version} is available`, {
					label: 'View',
					run: () => (update = found)
				});
			}),
			events.clipboardLinksEvent.listen((e) => {
				const links = e.payload.links;
				const what = links.length === 1 ? (links[0] ?? '') : `${links.length} links`;
				toasts.offer(`Copied: ${what}`, { label: 'Add', run: () => openAdd(links) });
			})
		];
		void showWhatsNew(false);
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

	/** Right-click: the menu acts on the selection, which the clicked row joins or becomes. */
	function onMenu(id: string, e: MouseEvent) {
		e.preventDefault();
		if (!selection.ids.has(id)) selection = sel.single(id);
		menu = { x: e.clientX, y: e.clientY };
	}

	async function copyLink(text: string) {
		try {
			await navigator.clipboard.writeText(text);
			toasts.info('Link copied');
		} catch (e) {
			toasts.error(errorMessage(e));
		}
	}

	function menuItems(rows: TaskRow[]): MenuItem[] {
		const [one] = rows;
		if (!one) return [];
		const items: MenuItem[] = [];
		const single = rows.length === 1;
		if (rows.some(canPause)) {
			items.push({ label: 'Pause', icon: Pause, run: () => rows.filter(canPause).forEach((r) => store.pause(r.id)) });
		}
		if (rows.some(canResume)) {
			const retry = rows.filter(canResume).every((r) => r.state.type === 'failed');
			items.push({
				label: retry ? 'Retry' : 'Resume',
				icon: retry ? RotateCcw : Play,
				run: () => rows.filter(canResume).forEach((r) => store.resume(r.id))
			});
		}
		if (single && one.state.type === 'queued') {
			items.push(
				{ label: 'Download next', icon: ArrowUpToLine, run: () => store.move(one.id, 'top') },
				{ label: 'Download last', icon: ArrowDownToLine, run: () => store.move(one.id, 'bottom') }
			);
		}
		if (single) {
			if (items.length > 0) items.push('separator');
			items.push({ label: 'Details', icon: Info, run: () => (detailsOpen = true) });
			if (one.state.type === 'completed' || one.state.type === 'seeding') {
				items.push({ label: 'Show in folder', icon: FolderOpen, run: () => store.reveal(one.id) });
			}
			if (one.source.includes(':')) {
				items.push({ label: 'Copy link', icon: Copy, run: () => copyLink(one.source) });
			}
		}
		items.push('separator', {
			label: single ? 'Remove…' : `Remove ${rows.length} downloads…`,
			icon: Trash2,
			danger: true,
			run: () => askRemove(rows.map((r) => r.id))
		});
		return items;
	}

	function onKeydown(e: KeyboardEvent) {
		// An open dropdown handles its own keys (Escape closes it).
		if (e.target instanceof HTMLOptionElement) return;
		const modalOpen =
			add.open || removing !== null || whatsNew !== null || update !== null || menu !== null;
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
	oncontextmenu={(e) => {
		// The webview's own menu (Reload, Save as…) makes no sense in an app; keep it
		// where text can be edited or copied.
		const selected = window.getSelection()?.toString() ?? '';
		if (!isTextField(e.target) && selected === '') e.preventDefault();
	}}
/>

<div class="flex h-full gap-3 p-3">
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

	<main class="relative flex min-w-0 flex-1 flex-col">
		{#if settingsOpen}
			<SettingsPage
				onsaved={(s: SettingsView) => (categories = s.categories)}
				onupdate={(u) => (update = u)}
				onshownotes={() => void showWhatsNew(true)}
			/>
		{:else}
			<header class="glass absolute inset-x-0 top-0 z-20 flex items-center gap-2 rounded-full px-2.5 py-2">
				<button class="btn-primary" onclick={() => openAdd()} title="New download (Ctrl+N)">
					<Plus size={14} /> Add
				</button>
				<div class="relative ml-1 max-w-72 min-w-24 flex-1">
					<Search size={14} class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-(--muted)" aria-hidden="true" />
					<input
						bind:this={searchInput}
						class="input rounded-full py-1.5 pl-8 text-xs"
						type="search"
						placeholder="Search (Ctrl+F)"
						aria-label="Search downloads"
						bind:value={search}
					/>
				</div>
				<select
					class="input w-36 shrink-0 rounded-full py-1.5 text-xs"
					aria-label="Sort downloads"
					bind:value={sort}
					onchange={() => {
						saveSort(sort);
						// Once the list has closed and given the focus back: shortcuts work again.
						setTimeout(() => list?.focus(), 0);
					}}
				>
					{#each SORTS as s (s.key)}<option value={s.key}>{s.label}</option>{/each}
				</select>
				<div class="ml-auto flex items-center gap-1.5">
					{#if selectedRows.length > 0}
						<span class="mr-1 text-xs whitespace-nowrap text-(--muted)" aria-live="polite">{selectedRows.length} selected</span>
						<button class="btn" aria-label="Pause" title="Pause" disabled={!selectedRows.some(canPause)} onclick={() => selectedRows.filter(canPause).forEach((r) => store.pause(r.id))}>
							<Pause size={13} /> <span class="hidden lg:inline">Pause</span>
						</button>
						<button class="btn" aria-label="Resume" title="Resume" disabled={!selectedRows.some(canResume)} onclick={() => selectedRows.filter(canResume).forEach((r) => store.resume(r.id))}>
							<Play size={13} /> <span class="hidden lg:inline">Resume</span>
						</button>
						<button class="btn" aria-label="Remove" title="Remove" onclick={() => askRemove(selectedRows.map((r) => r.id))}>
							<Trash2 size={13} /> <span class="hidden lg:inline">Remove</span>
						</button>
					{:else}
						<button class="btn" aria-label="Pause all" title="Pause all" disabled={!anyActive} onclick={() => store.pauseAll()}>
							<Pause size={13} /> <span class="hidden lg:inline">Pause all</span>
						</button>
						<button class="btn" aria-label="Resume all" title="Resume all" disabled={!anyPaused} onclick={() => store.resumeAll()}>
							<Play size={13} /> <span class="hidden lg:inline">Resume all</span>
						</button>
					{/if}
				</div>
			</header>

			{#if store.unavailable}
				<div role="alert" class="mt-16 rounded-2xl bg-red-500/15 p-3 text-sm text-red-800 dark:text-red-200">
					<p class="font-medium">The download engine is not running.</p>
					<p>{store.unavailable}</p>
					<p class="mt-1 text-xs">Close any running <code>persivio</code> command-line session, then restart the app.</p>
				</div>
			{/if}

			<div class="flex min-h-0 flex-1 gap-3">
				<div class="relative min-w-0 flex-1">
					{#if store.loaded && visible.length === 0 && !store.unavailable}
						<div class="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
							{#if store.tasks.length === 0}
								<span class="glass grid size-16 place-items-center rounded-3xl text-teal-700 dark:text-teal-300"><Download size={26} /></span>
								<p class="text-sm font-medium">No downloads yet</p>
								<p class="max-w-sm text-xs text-(--muted)">
									Paste a link with <span class="kbd">Ctrl+V</span>, drop links or .torrent files here, or click Add.
								</p>
								<button class="btn-primary mt-1" onclick={() => openAdd()}><Plus size={14} /> Add a download</button>
							{:else}
								<span class="text-(--muted)"><Inbox size={28} /></span>
								<p class="text-sm text-(--muted)">
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
							insetTop={60}
							insetBottom={52}
							onselect={onSelect}
							onmenu={onMenu}
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
					{#if store.tasks.length > 0}
						<div class="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex justify-center">
							<div class="pointer-events-auto"><StatusBar tasks={store.tasks} /></div>
						</div>
					{/if}
				</div>
				{#if detailsOpen && focusedTask}
					<div class="flex pt-[3.75rem]">
						<DetailDrawer task={focusedTask} {store} onclose={() => (detailsOpen = false)} />
					</div>
				{/if}
			</div>
		{/if}
	</main>
</div>

{#if dragging}
	<div class="pointer-events-none fixed inset-3 z-40 grid place-items-center rounded-3xl border-2 border-dashed border-teal-500 bg-teal-500/15 text-sm font-medium text-teal-900 backdrop-blur-sm dark:text-teal-100">
		Drop links or a .torrent file to add them
	</div>
{/if}

{#if menu && selectedRows.length > 0}
	<ContextMenu
		x={menu.x}
		y={menu.y}
		items={menuItems(selectedRows)}
		label="Actions for the selected downloads"
		onclose={() => {
			menu = null;
			list?.focus();
		}}
	/>
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
		<p class="text-sm">
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

<Modal
	open={whatsNew !== null}
	title="What's new in Persivio"
	onclose={() => {
		if (whatsNew) saveLastSeen(whatsNew.version);
		whatsNew = null;
	}}
>
	{#if whatsNew}<ReleaseNotes releases={whatsNew.releases} />{/if}
</Modal>

<UpdateDialog {update} onclose={() => (update = null)} />

<Toasts />
