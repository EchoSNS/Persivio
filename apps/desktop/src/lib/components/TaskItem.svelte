<script lang="ts">
	import type { TaskRow } from '#lib/stores/reducer.ts';
	import { isActive } from '#lib/stores/reducer.ts';
	import { formatBytes, formatEta, formatSpeed, percent } from '#lib/utils/format.ts';

	type Props = {
		task: TaskRow;
		onpause: () => void;
		onresume: () => void;
		oncancel: () => void;
		onremove: () => void;
		onreveal: () => void;
	};
	let { task, onpause, onresume, oncancel, onremove, onreveal }: Props = $props();

	const STATE_LABELS: Record<TaskRow['state']['type'], string> = {
		queued: 'Queued',
		probing: 'Connecting',
		fetching_metadata: 'Fetching metadata',
		downloading: 'Downloading',
		paused: 'Paused',
		verifying: 'Verifying',
		seeding: 'Seeding',
		completed: 'Completed',
		failed: 'Failed',
		cancelled: 'Cancelled'
	};

	let pct = $derived(percent(task.downloaded, task.total));
	let active = $derived(isActive(task.state));
	let canPause = $derived(active || task.state.type === 'queued');
	let canResume = $derived(
		task.state.type === 'paused' || (task.state.type === 'failed' && task.state.retryable)
	);
	let finished = $derived(['completed', 'cancelled', 'failed'].includes(task.state.type));
	let barColor = $derived(
		task.state.type === 'failed'
			? 'bg-red-500'
			: task.state.type === 'completed'
				? 'bg-emerald-500'
				: 'bg-teal-500'
	);
</script>

<li class="rounded-lg border border-zinc-200 bg-white p-3 dark:border-zinc-800 dark:bg-zinc-900">
	<div class="flex items-center gap-3">
		<span
			class="shrink-0 rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-zinc-600 uppercase dark:bg-zinc-800 dark:text-zinc-300"
		>
			{task.kind === 'torrent' ? 'Torrent' : 'HTTP'}
		</span>
		<div class="min-w-0 flex-1">
			<p class="truncate font-medium" title={task.name}>{task.name}</p>
			<p class="truncate text-xs text-zinc-500" title={task.source}>{task.source}</p>
		</div>
		<div class="flex shrink-0 gap-1">
			{#if canPause}
				<button class="btn" onclick={onpause}>Pause</button>
			{/if}
			{#if canResume}
				<button class="btn" onclick={onresume}>
					{task.state.type === 'failed' ? 'Retry' : 'Resume'}
				</button>
			{/if}
			{#if !finished}
				<button class="btn" onclick={oncancel}>Cancel</button>
			{/if}
			{#if task.state.type === 'completed' || task.state.type === 'seeding'}
				<button class="btn" onclick={onreveal}>Show in folder</button>
			{/if}
			{#if finished}
				<button class="btn" onclick={onremove}>Remove</button>
			{/if}
		</div>
	</div>

	<div
		class="mt-2 h-1.5 overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800"
		role="progressbar"
		aria-label="Progress of {task.name}"
		aria-valuemin={0}
		aria-valuemax={100}
		aria-valuenow={pct === null ? undefined : Math.round(pct)}
	>
		<div
			class="h-full rounded-full transition-[width] duration-300 ease-out {barColor}"
			style:width="{pct ?? 0}%"
		></div>
	</div>

	<div class="mt-1.5 flex flex-wrap gap-x-4 text-xs text-zinc-500">
		<span class={task.state.type === 'failed' ? 'text-red-600 dark:text-red-400' : ''}>
			{STATE_LABELS[task.state.type]}{task.state.type === 'failed' ? `: ${task.state.message}` : ''}
		</span>
		<span>
			{formatBytes(task.downloaded)}{task.total === null ? '' : ` of ${formatBytes(task.total)}`}
			{#if pct !== null}({pct.toFixed(1)}%){/if}
		</span>
		{#if active}
			<span>{formatSpeed(task.speedBps)}</span>
			<span>ETA {formatEta(task.etaSecs)}</span>
			{#if task.peers !== null}
				<span>{task.peers} peers</span>
			{:else}
				<span>{task.connections} connections</span>
			{/if}
			{#if task.uploadBps !== null}
				<span>Up {formatSpeed(task.uploadBps)}</span>
			{/if}
		{/if}
	</div>
</li>
