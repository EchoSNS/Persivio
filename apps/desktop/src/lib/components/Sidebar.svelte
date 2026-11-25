<script lang="ts">
	import {
		Activity,
		CircleCheck,
		CircleX,
		Clock,
		Download,
		Folder,
		Magnet,
		Settings
	} from '@lucide/svelte';
	import type { TaskRow } from '#lib/stores/reducer.ts';
	import StatusBar from './StatusBar.svelte';
	import { BUILTIN_FILTERS, filterKey, type Filter } from '#lib/stores/filters.ts';

	type Props = {
		tasks: TaskRow[];
		filter: Filter;
		counts: Map<string, number>;
		categories: string[];
		settingsOpen: boolean;
		onfilter: (f: Filter) => void;
		onsettings: () => void;
	};
	let { tasks, filter, counts, categories, settingsOpen, onfilter, onsettings }: Props = $props();

	const ICONS = {
		all: Download,
		active: Activity,
		queued: Clock,
		completed: CircleCheck,
		failed: CircleX,
		torrents: Magnet
	} as const;

	let current = $derived(settingsOpen ? null : filterKey(filter));
</script>

{#snippet entry(f: Filter, label: string, Icon: typeof Download)}
	{@const key = filterKey(f)}
	<li>
		<button
			class="nav-item"
			aria-current={current === key ? 'page' : undefined}
			onclick={() => onfilter(f)}
		>
			<Icon size={17} aria-hidden="true" />
			<span class="flex-1 truncate">{label}</span>
			{#if counts.get(key)}
				<span class="text-xs text-(--muted) tabular-nums">{counts.get(key)}</span>
			{/if}
		</button>
	</li>
{/snippet}

<nav
	class="flex w-52 shrink-0 flex-col"
	aria-label="Download lists"
>
	<div class="flex h-16 shrink-0 items-center gap-2.5 px-5" data-tauri-drag-region="deep">
		<img src="/persivio-logo.svg" alt="" class="size-8" width="32" height="32" draggable="false" />
		<span class="text-[15px] font-semibold tracking-tight select-none">Persivio</span>
	</div>

	<div class="min-h-0 flex-1 overflow-y-auto pt-3 pr-2">
		<ul class="space-y-1">
			{#each BUILTIN_FILTERS as f (f.type)}
				{@render entry({ type: f.type }, f.label, ICONS[f.type])}
			{/each}
		</ul>
		{#if categories.length > 0}
			<h2 class="mt-6 mb-1.5 px-5 text-xs font-medium text-(--muted)">
				Categories
			</h2>
			<ul class="space-y-1">
				{#each categories as name (name)}
					{@render entry({ type: 'category', name }, name, Folder)}
				{/each}
			</ul>
		{/if}
	</div>

	<div class="pt-2 pr-2 pb-5">
		<button
			class="nav-item"
			aria-current={settingsOpen ? 'page' : undefined}
			onclick={onsettings}
		>
			<Settings size={17} aria-hidden="true" /> Settings
		</button>
		{#if tasks.length > 0}<StatusBar {tasks} />{/if}
	</div>
</nav>
