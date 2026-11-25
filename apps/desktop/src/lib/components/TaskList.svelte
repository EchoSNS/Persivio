<!--
	Virtualized task list: only the rows in view (plus a few) exist in the DOM, so 500+
	tasks scroll as smoothly as 10. Rows have a fixed height; positions are applied
	through CSSOM (`style:`), which the strict CSP allows.
-->
<script lang="ts">
	import { createVirtualizer } from '@tanstack/svelte-virtual';
	import { get } from 'svelte/store';
	import type { TaskRow as Row } from '#lib/stores/reducer.ts';
	import type { Selection } from '#lib/stores/selection.ts';
	import TaskRow from './TaskRow.svelte';

	const ROW_HEIGHT = 74;

	type Props = {
		rows: Row[];
		selection: Selection;
		onselect: (id: string, e: MouseEvent) => void;
		onopen: (id: string) => void;
		onmenu: (id: string, e: MouseEvent) => void;
		onpause: (id: string) => void;
		onresume: (id: string) => void;
		onremove: (id: string) => void;
		onreveal: (id: string) => void;
		label: string;
		/** Space kept free for what floats over the top and bottom of the list. */
		insetTop?: number;
		insetBottom?: number;
	};
	let {
		rows,
		selection,
		onselect,
		onopen,
		onmenu,
		onpause,
		onresume,
		onremove,
		onreveal,
		label,
		insetTop = 0,
		insetBottom = 0
	}: Props = $props();

	let scroller: HTMLDivElement | undefined = $state();

	const virtualizer = createVirtualizer<HTMLDivElement, HTMLDivElement>({
		count: 0,
		getScrollElement: () => scroller ?? null,
		estimateSize: () => ROW_HEIGHT,
		overscan: 8
	});

	// `get` reads the virtualizer without subscribing: `setOptions` updates the store, and
	// a tracked read here would rerun this effect forever.
	$effect(() => {
		get(virtualizer).setOptions({
			count: rows.length,
			paddingStart: insetTop,
			paddingEnd: insetBottom,
			// Rows brought into view by the keyboard must not end up under the bars.
			scrollPaddingStart: insetTop,
			scrollPaddingEnd: insetBottom
		});
	});

	// Keep the keyboard focus row in view.
	$effect(() => {
		const index = selection.focus === null ? -1 : rows.findIndex((r) => r.id === selection.focus);
		if (index >= 0) get(virtualizer).scrollToIndex(index, { align: 'auto' });
	});

	/** Focus the list itself (for keyboard navigation). */
	export function focus() {
		scroller?.focus();
	}
</script>

<div
	bind:this={scroller}
	class="scroll-fade h-full overflow-y-auto focus:outline-none"
	role="listbox"
	aria-label={label}
	aria-multiselectable="true"
	aria-activedescendant={selection.focus ? `task-${selection.focus}` : undefined}
	tabindex="0"
>
	<div class="relative w-full" style:height="{$virtualizer.getTotalSize()}px">
		{#each $virtualizer.getVirtualItems() as item (rows[item.index]?.id ?? item.key)}
			{@const row = rows[item.index]}
			{#if row}
				<div
					class="absolute top-0 left-0 w-full"
					style:height="{ROW_HEIGHT}px"
					style:transform="translateY({item.start}px)"
				>
					<TaskRow
						task={row}
						selected={selection.ids.has(row.id)}
						focused={selection.focus === row.id}
						onselect={(e) => onselect(row.id, e)}
						onopen={() => onopen(row.id)}
						onmenu={(e) => onmenu(row.id, e)}
						onpause={() => onpause(row.id)}
						onresume={() => onresume(row.id)}
						onremove={() => onremove(row.id)}
						onreveal={() => onreveal(row.id)}
					/>
				</div>
			{/if}
		{/each}
	</div>
</div>
