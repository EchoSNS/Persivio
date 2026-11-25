<!--
	Minimize, maximize and close, for a window without a system title bar. Also placed
	inside every modal: an open modal makes the rest of the page inert, and the window must
	stay movable and closable. Like system buttons they take no keyboard focus (Alt+F4 and
	Alt+Space work).
-->
<script lang="ts">
	import { Copy, Minus, Square, X } from '@lucide/svelte';
	import { appWindow } from '#lib/stores/window.svelte.ts';

	let resizeLabel = $derived(appWindow.maximized ? 'Restore' : 'Maximize');
</script>

{#if appWindow.ownControls}
	<div class="window-controls" role="group" aria-label="Window">
		<button type="button" tabindex="-1" aria-label="Minimize" title="Minimize" onclick={appWindow.minimize}>
			<Minus size={15} strokeWidth={1.6} aria-hidden="true" />
		</button>
		<button
			type="button"
			tabindex="-1"
			aria-label={resizeLabel}
			title={resizeLabel}
			onclick={appWindow.toggleMaximize}
		>
			{#if appWindow.maximized}
				<Copy size={13} strokeWidth={1.6} class="-scale-x-100" aria-hidden="true" />
			{:else}
				<Square size={12} strokeWidth={1.6} aria-hidden="true" />
			{/if}
		</button>
		<button
			type="button"
			tabindex="-1"
			class="close"
			aria-label="Close window"
			title="Close"
			onclick={appWindow.close}
		>
			<X size={16} strokeWidth={1.6} aria-hidden="true" />
		</button>
	</div>
{/if}
