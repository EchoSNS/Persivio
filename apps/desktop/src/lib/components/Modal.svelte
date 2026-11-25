<!--
	A modal built on the native <dialog> element: the browser provides focus trapping,
	Escape to close, the top layer and `aria-modal`, with no inline styles (strict CSP).
-->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import { X } from '@lucide/svelte';
	import WindowControls from './WindowControls.svelte';

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
			// Without this the browser focuses the first control: Minimize.
			const first = dialog.querySelector<HTMLElement>('[data-autofocus]');
			(first ?? dialog.querySelector<HTMLElement>('[data-panel]'))?.focus();
		}
		if (!open && dialog.open) dialog.close();
	});
</script>

<dialog
	bind:this={dialog}
	class="modal"
	aria-labelledby={titleId}
	onclose={onclose}
	onclick={(e) => {
		// A click on the backdrop lands on the <dialog> itself.
		if (e.target === dialog) onclose();
	}}
>
	{#if open}
		<div class="absolute inset-x-0 top-0 flex h-11 justify-end" data-tauri-drag-region>
			<WindowControls />
		</div>
		<div class="modal-panel glass glass-strong {wide ? 'modal-wide' : ''}" data-panel tabindex="-1">
			<header
				class="flex items-center justify-between px-6 pt-5 pb-2"
			>
				<h2 id={titleId} class="text-base font-semibold">{title}</h2>
				<button class="icon-btn" aria-label="Close" onclick={onclose}><X size={16} /></button>
			</header>
			<div class="min-h-0 flex-1 overflow-y-auto px-6 pt-3 {footer ? 'pb-3' : 'pb-6'}">
				{@render children()}
			</div>
			{#if footer}
				<footer
					class="flex justify-end gap-2 px-6 pt-2 pb-5"
				>
					{@render footer()}
				</footer>
			{/if}
		</div>
	{/if}
</dialog>
