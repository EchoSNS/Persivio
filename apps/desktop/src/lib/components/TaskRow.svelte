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
		onpause: () => void;
		onresume: () => void;
		onremove: () => void;
		onreveal: () => void;
	};
	let { task, selected, focused, onselect, onopen, onpause, onresume, onremove, onreveal }: Props =
		$props();

	let pct = $derived(percent(task.downloaded, task.total));
	let active = $derived(isActive(task.state));
	let canPause = $derived(active || task.state.type === 'queued');
	let canResume = $derived(
		task.state.type === 'paused' || (task.state.type === 'failed' && task.state.retryable)
	);
	let finished = $derived(task.state.type === 'completed' || task.state.type === 'seeding');
	let barColor = $derived(
		task.state.type === 'failed'
			? 'bg-red-500'
			: task.state.type === 'completed'
				? 'bg-emerald-500'
				: task.state.type === 'paused' || task.state.type === 'queued'
					? 'bg-zinc-400 dark:bg-zinc-600'
					: 'bg-teal-500'
	);
	let sizeText = $derived(
		task.total === null
			? formatBytes(task.downloaded)
			: task.state.type === 'completed'
				? formatBytes(task.total)
				: `${formatBytes(task.downloaded)} / ${formatBytes(task.total)}`
	);

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
	class="group flex h-full cursor-default items-center gap-3 border-b border-zinc-100 px-3 select-none dark:border-zinc-800/70
		{selected ? 'bg-teal-50 dark:bg-teal-950/40' : 'hover:bg-zinc-100/70 dark:hover:bg-zinc-900'}
		{focused ? 'outline-2 -outline-offset-2 outline-teal-500/70' : ''}"
	onclick={onselect}
	ondblclick={onopen}
	onkeydown={() => {}}
>
	<span class="text-zinc-400" aria-hidden="true">
		{#if task.kind === 'torrent'}<Magnet size={16} />{:else}<Globe size={16} />{/if}
	</span>

	<div class="min-w-0 flex-1">
		<div class="flex items-center gap-2">
			<p class="truncate text-sm font-medium" title={task.name}>{task.name}</p>
			<StateBadge state={task.state} />
		</div>
		<div class="mt-1.5 flex items-center gap-3">
			<div class="h-1 flex-1 overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800">
				<div
					class="h-full rounded-full transition-[width] duration-300 ease-out {barColor}"
					style:width="{pct ?? 0}%"
				></div>
			</div>
			<span class="w-10 text-right text-[11px] text-zinc-500 tabular-nums">
				{pct === null ? '' : `${Math.floor(pct)}%`}
			</span>
		</div>
	</div>

	<div class="hidden w-28 text-right text-xs text-zinc-500 tabular-nums sm:block">{sizeText}</div>
	<div class="w-20 text-right text-xs tabular-nums {active ? 'text-zinc-700 dark:text-zinc-300' : 'text-zinc-400'}">
		{active ? formatSpeed(task.speedBps) : ''}
	</div>
	<div class="hidden w-16 text-right text-xs text-zinc-500 tabular-nums md:block">
		{active && task.state.type !== 'seeding' ? formatEta(task.etaSecs) : ''}
	</div>

	<div
		class="flex w-[5.5rem] justify-end gap-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 {selected
			? 'opacity-100'
			: ''}"
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
			<button class="icon-btn" aria-label="Show {task.name} in folder" title="Show in folder" onclick={action(onreveal)}>
				<FolderOpen size={15} />
			</button>
		{/if}
		<button class="icon-btn" aria-label="Remove {task.name}" title="Remove" onclick={action(onremove)}>
			<Trash2 size={15} />
		</button>
	</div>
</div>
