<script lang="ts">
	import { CircleAlert, Info, TriangleAlert, X } from '@lucide/svelte';
	import { toasts } from '#lib/stores/toasts.svelte.ts';

	const TONES = {
		info: 'text-teal-700 dark:text-teal-300',
		warn: 'text-amber-700 dark:text-amber-300',
		error: 'text-red-700 dark:text-red-300'
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
		<div class="glass glass-strong enter-up pointer-events-auto flex items-start gap-2.5 rounded-2xl p-3 text-sm">
			<Icon size={16} class="mt-0.5 shrink-0 {TONES[t.kind]}" aria-hidden="true" />
			<p class="flex-1 break-words">{t.message}</p>
			{#if t.action}
				<button
					class="btn-primary px-2 py-1"
					onclick={() => {
						t.action?.run();
						toasts.dismiss(t.id);
					}}
				>
					{t.action.label}
				</button>
			{/if}
			<button class="icon-btn size-5" aria-label="Dismiss" onclick={() => toasts.dismiss(t.id)}>
				<X size={14} />
			</button>
		</div>
	{/each}
</div>
