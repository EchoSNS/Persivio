<script lang="ts" module>
	import type { TaskStateView } from '#lib/api/bindings.ts';

	export const STATE_LABELS: Record<TaskStateView['type'], string> = {
		queued: 'Queued',
		probing: 'Connecting',
		fetching_metadata: 'Metadata',
		downloading: 'Downloading',
		paused: 'Paused',
		verifying: 'Verifying',
		seeding: 'Seeding',
		completed: 'Completed',
		failed: 'Failed',
		cancelled: 'Cancelled'
	};

	const TONES: Record<TaskStateView['type'], string> = {
		queued: 'bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300',
		probing: 'bg-sky-100 text-sky-700 dark:bg-sky-950 dark:text-sky-300',
		fetching_metadata: 'bg-sky-100 text-sky-700 dark:bg-sky-950 dark:text-sky-300',
		downloading: 'bg-teal-100 text-teal-800 dark:bg-teal-950 dark:text-teal-300',
		paused: 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300',
		verifying: 'bg-sky-100 text-sky-700 dark:bg-sky-950 dark:text-sky-300',
		seeding: 'bg-violet-100 text-violet-700 dark:bg-violet-950 dark:text-violet-300',
		completed: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300',
		failed: 'bg-red-100 text-red-700 dark:bg-red-950 dark:text-red-300',
		cancelled: 'bg-zinc-100 text-zinc-500 dark:bg-zinc-800 dark:text-zinc-400'
	};
</script>

<script lang="ts">
	let { state }: { state: TaskStateView } = $props();
</script>

<span
	class="inline-flex shrink-0 items-center rounded px-1.5 py-0.5 text-[10px] font-semibold {TONES[
		state.type
	]}"
	title={state.type === 'failed' ? state.message : undefined}
>
	{STATE_LABELS[state.type]}
</span>
