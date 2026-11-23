<script lang="ts">
	import { ArrowDown, ArrowUp, FolderOpen, KeyRound, X } from '@lucide/svelte';
	import { commands, errorMessage, type TaskDetailView } from '#lib/api/index.ts';
	import type { TaskRow } from '#lib/stores/reducer.ts';
	import { isActive } from '#lib/stores/reducer.ts';
	import type { TaskStore } from '#lib/stores/tasks.svelte.ts';
	import { formatBytes, formatEta, formatSpeed, percent } from '#lib/utils/format.ts';
	import FileSelector from './FileSelector.svelte';
	import SegmentBar from './SegmentBar.svelte';
	import Sparkline from './Sparkline.svelte';
	import StateBadge from './StateBadge.svelte';

	type Props = { task: TaskRow; store: TaskStore; onclose: () => void };
	let { task, store, onclose }: Props = $props();

	let detail = $state<TaskDetailView | null>(null);
	let error = $state<string | null>(null);
	let selection = $state(new Set<number>());
	let selectionDirty = $state(false);

	let active = $derived(isActive(task.state));
	let pct = $derived(percent(task.downloaded, task.total));
	let waiting = $derived(task.state.type === 'queued' || task.state.type === 'paused');

	async function load(id: string) {
		try {
			const d = await commands.getTaskDetail(id);
			if (id !== task.id) return; // the user moved on meanwhile
			detail = d;
			error = null;
			if (!selectionDirty) selection = new Set(d.files.filter((f) => f.selected).map((f) => f.index));
		} catch (e) {
			error = errorMessage(e);
		}
	}

	// Load when another task is shown, and again when its metadata (name, size, a
	// magnet's file list) arrives.
	let shownId: string | null = null;
	$effect(() => {
		const id = task.id;
		void task.name;
		void task.total;
		if (id !== shownId) {
			shownId = id;
			selectionDirty = false;
			detail = null;
		}
		void load(id);
	});
	// Refresh the segments once a second, only while the task runs.
	$effect(() => {
		if (!active) return;
		const id = task.id;
		const timer = setInterval(() => void load(id), 1000);
		return () => clearInterval(timer);
	});

	async function applySelection() {
		if (!detail) return;
		const all = selection.size === detail.files.length;
		if (await store.selectFiles(task.id, all ? null : [...selection].sort((a, b) => a - b))) {
			selectionDirty = false;
		}
	}

	const date = (ms: number | null) => (ms === null ? '—' : new Date(ms).toLocaleString());
</script>

<aside
	class="flex h-full w-[22rem] shrink-0 flex-col border-l border-zinc-200 bg-white dark:border-zinc-800 dark:bg-zinc-900"
	aria-label="Download details"
>
	<header class="flex items-start gap-2 border-b border-zinc-200 px-4 py-3 dark:border-zinc-800">
		<div class="min-w-0 flex-1">
			<h2 class="text-sm font-semibold break-words">{task.name}</h2>
			<div class="mt-1 flex items-center gap-2"><StateBadge state={task.state} /></div>
		</div>
		<button class="icon-btn" aria-label="Close details" onclick={onclose}><X size={16} /></button>
	</header>

	<div class="flex-1 space-y-5 overflow-y-auto px-4 py-4 text-sm">
		{#if task.state.type === 'failed'}
			<p class="rounded-md bg-red-50 p-2.5 text-xs text-red-800 dark:bg-red-950 dark:text-red-200">
				{task.state.message}
			</p>
		{/if}

		<section aria-label="Progress" class="space-y-2">
			<div class="flex justify-between text-xs text-zinc-500 tabular-nums">
				<span>
					{formatBytes(task.downloaded)}{task.total === null ? '' : ` of ${formatBytes(task.total)}`}
				</span>
				<span>{pct === null ? '' : `${pct.toFixed(1)}%`}</span>
			</div>
			{#if detail && detail.segments.length > 0 && task.total}
				<SegmentBar segments={detail.segments} total={task.total} />
				<p class="text-[11px] text-zinc-500">{detail.segments.length} segments</p>
			{/if}
			{#if active}
				<dl class="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
					<dt class="text-zinc-500">Speed</dt>
					<dd class="text-right tabular-nums">{formatSpeed(task.speedBps)}</dd>
					<dt class="text-zinc-500">Time left</dt>
					<dd class="text-right tabular-nums">{formatEta(task.etaSecs)}</dd>
					{#if task.peers !== null}
						<dt class="text-zinc-500">Peers</dt>
						<dd class="text-right tabular-nums">{task.peers}</dd>
						<dt class="text-zinc-500">Upload</dt>
						<dd class="text-right tabular-nums">{formatSpeed(task.uploadBps ?? 0)}</dd>
					{:else}
						<dt class="text-zinc-500">Connections</dt>
						<dd class="text-right tabular-nums">{task.connections}</dd>
					{/if}
				</dl>
				<Sparkline samples={store.speeds(task.id)} />
			{/if}
		</section>

		{#if detail && task.kind === 'torrent' && detail.files.length > 0}
			<section aria-label="Files" class="space-y-2">
				<h3 class="text-xs font-semibold text-zinc-500 uppercase">Files</h3>
				<FileSelector
					files={detail.files}
					selected={selection}
					onchange={(s) => {
						selection = s;
						selectionDirty = true;
					}}
				/>
				{#if selectionDirty}
					<button class="btn-primary" disabled={selection.size === 0} onclick={applySelection}>
						Apply selection
					</button>
				{/if}
			</section>
		{/if}

		<section aria-label="Information">
			<dl class="space-y-2 text-xs">
				<div>
					<dt class="text-zinc-500">Source</dt>
					<dd class="break-all select-text">{task.source}</dd>
				</div>
				{#if detail?.finalUrl && detail.finalUrl !== task.source}
					<div>
						<dt class="text-zinc-500">Final URL</dt>
						<dd class="break-all select-text">{detail.finalUrl}</dd>
					</div>
				{/if}
				<div>
					<dt class="text-zinc-500">Saved to</dt>
					<dd class="break-all select-text">{detail?.path ?? task.destDir}</dd>
				</div>
				{#if detail?.checksum}
					<div>
						<dt class="text-zinc-500">Expected checksum</dt>
						<dd class="font-mono break-all select-text">{detail.checksum}</dd>
					</div>
				{/if}
				{#if detail?.sendsCredentials}
					<div class="flex items-center gap-1.5 text-zinc-500">
						<KeyRound size={12} aria-hidden="true" /> Sent with cookies or custom headers
					</div>
				{/if}
				{#if detail?.connections || detail?.speedLimitBps}
					<div class="flex gap-4">
						{#if detail.connections}<span><span class="text-zinc-500">Connections</span> {detail.connections}</span>{/if}
						{#if detail.speedLimitBps}<span><span class="text-zinc-500">Limit</span> {formatSpeed(detail.speedLimitBps)}</span>{/if}
					</div>
				{/if}
				<div class="flex gap-4">
					<span><span class="text-zinc-500">Added</span> {date(task.createdAt)}</span>
				</div>
				{#if task.completedAt}
					<div><span class="text-zinc-500">Completed</span> {date(task.completedAt)}</div>
				{/if}
				{#if task.category}
					<div><span class="text-zinc-500">Category</span> {task.category}</div>
				{/if}
			</dl>
		</section>

		{#if error}
			<p class="field-error">{error}</p>
		{/if}
	</div>

	<footer class="flex flex-wrap gap-2 border-t border-zinc-200 px-4 py-3 dark:border-zinc-800">
		{#if task.state.type === 'completed' || task.state.type === 'seeding'}
			<button class="btn" onclick={() => store.reveal(task.id)}><FolderOpen size={14} /> Show in folder</button>
		{/if}
		{#if waiting}
			<button class="btn" onclick={() => store.move(task.id, 'up')} aria-label="Move up in the queue">
				<ArrowUp size={14} /> Up
			</button>
			<button class="btn" onclick={() => store.move(task.id, 'down')} aria-label="Move down in the queue">
				<ArrowDown size={14} /> Down
			</button>
		{/if}
	</footer>
</aside>
