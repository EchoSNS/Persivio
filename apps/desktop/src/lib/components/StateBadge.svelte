<script lang="ts" module>
	import type { TaskStateView } from '#lib/api/bindings.ts';

	export const STATE_LABELS: Record<TaskStateView['type'], string> = {
		queued: 'Queued',
		probing: 'Connecting',
		fetching_metadata: 'Metadata',
		checking: 'Checking files',
		downloading: 'Downloading',
		paused: 'Paused',
		verifying: 'Verifying',
		seeding: 'Seeding',
		completed: 'Completed',
		failed: 'Failed',
		cancelled: 'Cancelled'
	};

	const NEUTRAL = 'bg-zinc-500/15 text-zinc-700 dark:text-zinc-300';
	const WORKING = 'bg-sky-500/18 text-sky-800 dark:text-sky-200';
	const TONES: Record<TaskStateView['type'], string> = {
		queued: NEUTRAL,
		probing: WORKING,
		fetching_metadata: WORKING,
		checking: WORKING,
		downloading: 'bg-teal-500/18 text-teal-800 dark:text-teal-200',
		paused: 'bg-amber-500/22 text-amber-900 dark:text-amber-200',
		verifying: WORKING,
		seeding: 'bg-violet-500/18 text-violet-800 dark:text-violet-200',
		completed: 'bg-emerald-500/18 text-emerald-800 dark:text-emerald-200',
		failed: 'bg-red-500/18 text-red-800 dark:text-red-200',
		cancelled: NEUTRAL
	};
</script>

<script lang="ts">
	let { state, title }: { state: TaskStateView; title?: string } = $props();
</script>

<span
	class="inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-[10px] font-semibold {TONES[
		state.type
	]}"
	title={state.type === 'failed' ? state.message : title}
>
	{STATE_LABELS[state.type]}
</span>
