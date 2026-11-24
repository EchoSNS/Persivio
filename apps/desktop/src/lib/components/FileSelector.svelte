<!-- Checkbox list of a torrent's files, with select all / none and the chosen size. -->
<script lang="ts">
	import type { FileView } from '#lib/api/bindings.ts';
	import { formatBytes } from '#lib/utils/format.ts';

	type Props = { files: FileView[]; selected: Set<number>; onchange: (s: Set<number>) => void };
	let { files, selected, onchange }: Props = $props();

	let chosenSize = $derived(
		files.reduce((sum, f) => sum + (selected.has(f.index) ? f.size : 0), 0)
	);

	function toggle(index: number, on: boolean) {
		const next = new Set(selected);
		if (on) next.add(index);
		else next.delete(index);
		onchange(next);
	}
</script>

<div class="space-y-2">
	<div class="flex items-center justify-between text-xs text-(--muted)">
		<span>{selected.size} of {files.length} files · {formatBytes(chosenSize)}</span>
		<span class="flex gap-2">
			<button class="underline hover:text-zinc-950 dark:hover:text-white" onclick={() => onchange(new Set(files.map((f) => f.index)))}>
				All
			</button>
			<button class="underline hover:text-zinc-950 dark:hover:text-white" onclick={() => onchange(new Set())}>
				None
			</button>
		</span>
	</div>
	<ul class="max-h-56 overflow-y-auto rounded-xl border border-(--line) bg-(--field)" aria-label="Files">
		{#each files as f (f.index)}
			<li class="border-b border-(--line) last:border-0">
				<label class="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-xs hover:bg-(--row-hover)">
					<input
						type="checkbox"
						class="accent-teal-600"
						checked={selected.has(f.index)}
						onchange={(e) => toggle(f.index, e.currentTarget.checked)}
					/>
					<span class="min-w-0 flex-1 truncate" title={f.path}>{f.path}</span>
					<span class="text-(--muted) tabular-nums">{formatBytes(f.size)}</span>
				</label>
			</li>
		{/each}
	</ul>
</div>
