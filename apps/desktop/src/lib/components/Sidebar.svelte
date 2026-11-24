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
			class="nav-item"
			aria-current={current === key ? 'page' : undefined}
			onclick={() => onfilter(f)}
		>
			<Icon size={16} aria-hidden="true" />
			<span class="flex-1 truncate">{label}</span>
			{#if counts.get(key)}
				<span class="text-xs text-(--muted) tabular-nums">{counts.get(key)}</span>
			{/if}
		</button>
	</li>
{/snippet}

<nav
	class="glass flex w-52 shrink-0 flex-col rounded-3xl"
	aria-label="Download lists"
>
	<div class="flex items-center gap-2 px-4 py-3.5">
		<span class="grid size-7 place-items-center rounded-xl border border-white/30 bg-linear-to-br from-teal-500 to-cyan-600 text-white shadow-md shadow-teal-600/30" aria-hidden="true">
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
			<h2 class="mt-5 mb-1 px-2.5 text-[11px] font-semibold tracking-wide text-(--muted) uppercase">
				Categories
			</h2>
			<ul class="space-y-0.5">
				{#each categories as name (name)}
					{@render entry({ type: 'category', name }, name, Folder)}
				{/each}
			</ul>
		{/if}
	</div>

	<div class="border-t border-(--line) p-2">
		<button
			class="nav-item"
			aria-current={settingsOpen ? 'page' : undefined}
			onclick={onsettings}
		>
			<Settings size={16} aria-hidden="true" /> Settings
		</button>
	</div>
</nav>
