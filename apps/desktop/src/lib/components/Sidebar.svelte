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
	import { BUILTIN_FILTERS, filterKey, type Filter } from '#lib/stores/filters.ts';

	type Props = {
		filter: Filter;
		counts: Map<string, number>;
		categories: string[];
		settingsOpen: boolean;
		onfilter: (f: Filter) => void;
		onsettings: () => void;
	};
	let { filter, counts, categories, settingsOpen, onfilter, onsettings }: Props = $props();

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
			class="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm focus-visible:ring-2 focus-visible:ring-teal-500/60 focus-visible:outline-none
				{current === key
				? 'bg-teal-600/10 font-medium text-teal-800 dark:bg-teal-400/10 dark:text-teal-300'
				: 'text-zinc-700 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-800'}"
			aria-current={current === key ? 'page' : undefined}
			onclick={() => onfilter(f)}
		>
			<Icon size={16} aria-hidden="true" />
			<span class="flex-1 truncate">{label}</span>
			<span class="text-xs text-zinc-500 tabular-nums">{counts.get(key) ?? 0}</span>
		</button>
	</li>
{/snippet}

<nav
	class="flex h-full w-52 shrink-0 flex-col border-r border-zinc-200 bg-zinc-100/60 dark:border-zinc-800 dark:bg-zinc-900/60"
	aria-label="Download lists"
>
	<div class="flex items-center gap-2 px-4 py-3.5">
		<span class="grid size-6 place-items-center rounded-md bg-teal-600 text-white" aria-hidden="true">
			<Download size={14} strokeWidth={2.5} />
		</span>
		<span class="text-sm font-semibold tracking-tight">Persivio</span>
	</div>

	<div class="flex-1 overflow-y-auto px-2">
		<ul class="space-y-0.5">
			{#each BUILTIN_FILTERS as f (f.type)}
				{@render entry({ type: f.type }, f.label, ICONS[f.type])}
			{/each}
		</ul>
		{#if categories.length > 0}
			<h2 class="mt-5 mb-1 px-2.5 text-[11px] font-semibold tracking-wide text-zinc-500 uppercase">
				Categories
			</h2>
			<ul class="space-y-0.5">
				{#each categories as name (name)}
					{@render entry({ type: 'category', name }, name, Folder)}
				{/each}
			</ul>
		{/if}
	</div>

	<div class="border-t border-zinc-200 p-2 dark:border-zinc-800">
		<button
			class="flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm focus-visible:ring-2 focus-visible:ring-teal-500/60 focus-visible:outline-none
				{settingsOpen
				? 'bg-teal-600/10 font-medium text-teal-800 dark:bg-teal-400/10 dark:text-teal-300'
				: 'text-zinc-700 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-800'}"
			aria-current={settingsOpen ? 'page' : undefined}
			onclick={onsettings}
		>
			<Settings size={16} aria-hidden="true" /> Settings
		</button>
	</div>
</nav>
