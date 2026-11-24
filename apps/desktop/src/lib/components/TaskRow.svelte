<script lang="ts" module>
	/** What the progress bar of a task shows. */
	export const CHECKING_HINT =
		'Reading the data already on disk to make sure it is intact. This is not downloading.';
</script>

<script lang="ts">
	import { FolderOpen, Globe, Magnet, Pause, Play, RotateCcw, Trash2 } from '@lucide/svelte';
	import type { TaskRow } from '#lib/stores/reducer.ts';
	import { isActive } from '#lib/stores/reducer.ts';
	import { formatBytes, formatEta, formatSpeed, percent } from '#lib/utils/format.ts';
	import StateBadge from './StateBadge.svelte';

	type Props = {
		task: TaskRow;
		selected: boolean;
		focused: boolean;
		onselect: (e: MouseEvent) => void;
		onopen: () => void;
		onmenu: (e: MouseEvent) => void;
		onpause: () => void;
		onresume: () => void;
		onremove: () => void;
		onreveal: () => void;
	};
	let {
		task,
		selected,
		focused,
		onselect,
		onopen,
		onmenu,
		onpause,
		onresume,
		onremove,
		onreveal
	}: Props = $props();

	let checking = $derived(task.state.type === 'checking');
	let pct = $derived(checking ? task.checkPercent : percent(task.downloaded, task.total));
	let active = $derived(isActive(task.state));
	let canPause = $derived(active || task.state.type === 'queued');
	let canResume = $derived(
		task.state.type === 'paused' || (task.state.type === 'failed' && task.state.retryable)
	);
	let finished = $derived(task.state.type === 'completed' || task.state.type === 'seeding');
	// Working, with nothing to measure yet.
	let indeterminate = $derived(active && pct === null);
	let barColor = $derived.by(() => {
		switch (task.state.type) {
			case 'failed':
				return 'bg-red-500';
			case 'completed':
				return 'bg-emerald-500';
			case 'seeding':
				return 'bg-violet-500';
			case 'paused':
			case 'queued':
			case 'cancelled':
				return 'bg-zinc-400 dark:bg-zinc-500';
			case 'checking':
				return 'bg-sky-500 bar-stripes';
			default:
				return 'bg-linear-to-r from-teal-500 to-cyan-400';
		}
	});
	let speedText = $derived(
		!active || checking || task.state.type === 'probing' || task.state.type === 'fetching_metadata'
			? ''
			: formatSpeed(task.state.type === 'seeding' ? (task.uploadBps ?? 0) : task.speedBps)
	);
	let detailText = $derived.by(() => {
		if (task.state.type === 'failed') return task.state.message;
		const size = task.total === null ? '' : formatBytes(task.total);
		if (task.state.type === 'completed' || task.state.type === 'seeding') return size;
		if (checking) return `Reading ${size} from disk, not downloading`;
		if (task.total === null) return task.downloaded > 0 ? formatBytes(task.downloaded) : '';
		const parts = [`${formatBytes(task.downloaded)} of ${size}`];
		if (active && task.etaSecs !== null) parts.push(`${formatEta(task.etaSecs)} left`);
		return parts.join(' · ');
	});

	// Row action buttons must not change the selection or open details.
	function action(fn: () => void) {
		return (e: MouseEvent) => {
			e.stopPropagation();
			fn();
		};
	}
</script>

<div
	id="task-{task.id}"
	role="option"
	aria-selected={selected}
	tabindex="-1"
	class="group mx-2 flex h-[calc(100%-4px)] cursor-default items-center gap-3 rounded-2xl px-3 transition-colors duration-100 select-none
		{selected ? 'bg-(--row-selected)' : 'hover:bg-(--row-hover)'}
		{focused ? 'outline-2 -outline-offset-2 outline-(--ring)/70' : ''}"
	onclick={onselect}
	ondblclick={onopen}
	oncontextmenu={onmenu}
	onkeydown={() => {}}
>
	<span
		class="grid size-9 shrink-0 place-items-center rounded-xl border border-(--line) bg-(--control) text-(--muted)"
		aria-hidden="true"
	>
		{#if task.kind === 'torrent'}<Magnet size={16} />{:else}<Globe size={16} />{/if}
	</span>

	<div class="min-w-0 flex-1">
		<div class="flex items-center gap-2">
			<p class="truncate text-sm font-medium" title={task.name}>{task.name}</p>
			<StateBadge state={task.state} title={checking ? CHECKING_HINT : undefined} />
			<span class="ml-auto shrink-0 pl-2 text-xs font-semibold tabular-nums">{speedText}</span>
		</div>
		<div
			class="mt-1.5 h-1.5 overflow-hidden rounded-full bg-(--track)"
			role="progressbar"
			aria-label="{checking ? 'Check' : 'Download'} progress of {task.name}"
			aria-valuemin="0"
			aria-valuemax="100"
			aria-valuenow={pct === null ? undefined : Math.floor(pct)}
		>
			{#if indeterminate}
				<div class="bar bar-indeterminate {barColor}"></div>
			{:else}
				<div class="bar {barColor}" style:width="{pct ?? 0}%"></div>
			{/if}
		</div>
		<div class="mt-1 flex gap-3 text-[11px] text-(--muted) tabular-nums">
			<span class="min-w-0 truncate">{detailText}</span>
			<span class="ml-auto shrink-0">{pct === null ? '' : `${Math.floor(pct)}%`}</span>
		</div>
	</div>

	<!-- Takes no room until needed, but stays reachable with the keyboard. -->
	<div
		class="flex justify-end gap-0.5 overflow-hidden group-focus-within:w-auto group-hover:w-auto
			{selected ? 'w-auto' : 'w-0'}"
	>
		{#if canPause}
			<button class="icon-btn" aria-label="Pause {task.name}" title="Pause" onclick={action(onpause)}>
				<Pause size={15} />
			</button>
		{:else if canResume}
			<button
				class="icon-btn"
				aria-label="{task.state.type === 'failed' ? 'Retry' : 'Resume'} {task.name}"
				title={task.state.type === 'failed' ? 'Retry' : 'Resume'}
				onclick={action(onresume)}
			>
				{#if task.state.type === 'failed'}<RotateCcw size={15} />{:else}<Play size={15} />{/if}
			</button>
		{/if}
		{#if finished}
			<button
				class="icon-btn"
				aria-label="Show {task.name} in folder"
				title="Show in folder"
				onclick={action(onreveal)}
			>
				<FolderOpen size={15} />
			</button>
		{/if}
		<button class="icon-btn" aria-label="Remove {task.name}" title="Remove" onclick={action(onremove)}>
			<Trash2 size={15} />
		</button>
	</div>
</div>
