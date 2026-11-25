<!-- Totals over every download, at the foot of the sidebar. -->
<script lang="ts">
	import { ArrowDown, ArrowUp } from '@lucide/svelte';
	import { isActive, type TaskRow } from '#lib/stores/reducer.ts';
	import { formatSpeed } from '#lib/utils/format.ts';

	let { tasks }: { tasks: TaskRow[] } = $props();

	let totals = $derived.by(() => {
		let down = 0;
		let up = 0;
		let active = 0;
		let queued = 0;
		for (const t of tasks) {
			down += t.speedBps;
			up += t.uploadBps ?? 0;
			if (isActive(t.state)) active += 1;
			else if (t.state.type === 'queued') queued += 1;
		}
		return { down, up, active, queued };
	});
</script>

<div
	class="mt-3 ml-5 grid gap-1.5 text-xs tabular-nums"
	role="group"
	aria-label="Totals"
>
	<span class="inline-flex items-center gap-1.5" title="Total download speed">
		<ArrowDown size={13} class="text-teal-700 dark:text-teal-300" aria-hidden="true" />
		<span class="sr-only">Download speed</span>
		<span class="font-medium">{formatSpeed(totals.down)}</span>
	</span>
	<span class="inline-flex items-center gap-1.5" title="Total upload speed">
		<ArrowUp size={13} class="text-violet-700 dark:text-violet-300" aria-hidden="true" />
		<span class="sr-only">Upload speed</span>
		<span class="font-medium">{formatSpeed(totals.up)}</span>
	</span>
	<span class="text-(--muted)">
		{totals.active} active{totals.queued > 0 ? `, ${totals.queued} waiting` : ''}
	</span>
</div>
