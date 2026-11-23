<script lang="ts">
	import { CircleAlert, Info, TriangleAlert, X } from '@lucide/svelte';
	import { toasts } from '#lib/stores/toasts.svelte.ts';

	const TONES = {
		info: 'border-zinc-200 bg-white text-zinc-800 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100',
		warn: 'border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100',
		error: 'border-red-300 bg-red-50 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100'
	};
	const ICONS = { info: Info, warn: TriangleAlert, error: CircleAlert };
</script>

<!-- role=status + aria-live: screen readers announce new messages without moving focus. -->
<div
	class="pointer-events-none fixed right-4 bottom-4 z-50 flex w-80 flex-col gap-2"
	role="status"
	aria-live="polite"
>
	{#each toasts.items as t (t.id)}
		{@const Icon = ICONS[t.kind]}
		<div class="pointer-events-auto flex items-start gap-2 rounded-lg border p-3 text-sm shadow-lg {TONES[t.kind]}">
			<Icon size={16} class="mt-0.5 shrink-0" aria-hidden="true" />
			<p class="flex-1 break-words">{t.message}</p>
			<button class="icon-btn size-5" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}>
				<X size={14} />
			</button>
		</div>
	{/each}
</div>
