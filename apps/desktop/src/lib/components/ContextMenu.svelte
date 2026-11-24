<!--
	A right-click menu. Follows the ARIA menu pattern: focus moves into it, arrow keys walk
	the items, Escape or a click elsewhere closes it and returns focus.
-->
<script lang="ts" module>
	import type { Component } from 'svelte';

	export type MenuItem =
		| { label: string; icon: Component; run: () => void; danger?: boolean; disabled?: boolean }
		| 'separator';
</script>

<script lang="ts">
	type Props = { x: number; y: number; items: MenuItem[]; label: string; onclose: () => void };
	let { x, y, items, label, onclose }: Props = $props();

	let menu: HTMLDivElement | undefined = $state();
	let position = $state({ left: 0, top: 0 });

	const enabled = () =>
		[...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];

	// Keep the menu inside the window, then move focus into it. `preventScroll`: it is
	// positioned in this same step.
	$effect(() => {
		if (!menu) return;
		position = {
			left: Math.max(8, Math.min(x, window.innerWidth - menu.offsetWidth - 8)),
			top: Math.max(8, Math.min(y, window.innerHeight - menu.offsetHeight - 8))
		};
		enabled()[0]?.focus({ preventScroll: true });
	});

	function onkeydown(e: KeyboardEvent) {
		// Other keys (Enter, Space) reach the focused button; none reach the list behind.
		e.stopPropagation();
		const buttons = enabled();
		const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
		const go = (i: number) => {
			e.preventDefault();
			buttons[(i + buttons.length) % buttons.length]?.focus();
		};
		if (e.key === 'ArrowDown') go(at + 1);
		else if (e.key === 'ArrowUp') go(at - 1);
		else if (e.key === 'Home') go(0);
		else if (e.key === 'End') go(buttons.length - 1);
		else if (e.key === 'Escape' || e.key === 'Tab') {
			e.preventDefault();
			onclose();
		}
	}
</script>

<svelte:window onblur={onclose} onresize={onclose} />

<!-- Catches clicks outside the menu. -->
<div
	class="fixed inset-0 z-40"
	role="presentation"
	onpointerdown={onclose}
	oncontextmenu={(e) => {
		e.preventDefault();
		onclose();
	}}
></div>

<div
	bind:this={menu}
	class="glass glass-strong enter-up fixed z-50 min-w-48 rounded-2xl p-1.5"
	style:left="{position.left}px"
	style:top="{position.top}px"
	role="menu"
	aria-label={label}
	tabindex="-1"
	{onkeydown}
>
	{#each items as item, i (i)}
		{#if item === 'separator'}
			<div class="mx-2 my-1 border-t border-(--line)" role="separator"></div>
		{:else}
			<button
				class="menu-item {item.danger ? 'text-red-700 dark:text-red-300' : ''}"
				role="menuitem"
				tabindex="-1"
				disabled={item.disabled}
				onclick={() => {
					onclose();
					item.run();
				}}
			>
				<item.icon size={15} aria-hidden="true" />
				{item.label}
			</button>
		{/if}
	{/each}
</div>
