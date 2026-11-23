<script lang="ts">
	import { sparklinePath } from '#lib/stores/history.ts';
	import { formatSpeed } from '#lib/utils/format.ts';

	let { samples }: { samples: readonly number[] } = $props();

	const W = 280;
	const H = 48;
	let path = $derived(sparklinePath(samples, W, H));
	let peak = $derived(samples.length ? Math.max(...samples) : 0);
</script>

<figure class="m-0">
	<svg
		viewBox="0 0 {W} {H}"
		preserveAspectRatio="none"
		class="h-12 w-full text-teal-500"
		role="img"
		aria-label="Download speed over the last {samples.length} samples, peak {formatSpeed(peak)}"
	>
		{#if path}
			<path d="{path} L{W} {H} L0 {H} Z" class="fill-teal-500/15" />
			<path d={path} fill="none" stroke="currentColor" stroke-width="1.5" vector-effect="non-scaling-stroke" />
		{:else}
			<line x1="0" y1={H - 1} x2={W} y2={H - 1} class="stroke-zinc-300 dark:stroke-zinc-700" />
		{/if}
	</svg>
	<figcaption class="mt-1 flex justify-between text-[10px] text-zinc-500">
		<span>last {Math.round(samples.length / 4)} s</span>
		<span>peak {formatSpeed(peak)}</span>
	</figcaption>
</figure>
