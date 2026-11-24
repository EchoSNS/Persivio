<!--
	A modal built on the native <dialog> element: the browser provides focus trapping,
	Escape to close, the top layer and `aria-modal`, with no inline styles (strict CSP).
-->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import { X } from '@lucide/svelte';

	type Props = {
		open: boolean;
		title: string;
		onclose: () => void;
		children: Snippet;
		footer?: Snippet;
		wide?: boolean;
	};
	let { open, title, onclose, children, footer, wide = false }: Props = $props();

	let dialog: HTMLDialogElement | undefined = $state();
	const titleId = `modal-${Math.random().toString(36).slice(2)}`;

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) {
			dialog.showModal();
			// Without this the browser focuses the first control, the Close button.
			dialog.querySelector<HTMLElement>('[data-autofocus]')?.focus();
		}
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	class="modal glass glass-strong {wide ? 'w-[min(52rem,calc(100vw-2rem))]' : ''}"
	aria-labelledby={titleId}
	onclose={onclose}
	onclick={(e) => {
		// A click on the backdrop lands on the <dialog> itself.
		if (e.target === dialog) onclose();
	}}
>
	{#if open}
		<div class="flex max-h-[85vh] flex-col">
			<header
				class="flex items-center justify-between border-b border-(--line) px-5 py-3.5"
			>
				<h2 id={titleId} class="text-sm font-semibold">{title}</h2>
				<button class="icon-btn" aria-label="Close" onclick={onclose}><X size={16} /></button>
			</header>
			<div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
				{@render children()}
			</div>
			{#if footer}
				<footer
					class="flex justify-end gap-2 border-t border-(--line) px-5 py-3.5"
				>
					{@render footer()}
				</footer>
			{/if}
		</div>
	{/if}
</dialog>
