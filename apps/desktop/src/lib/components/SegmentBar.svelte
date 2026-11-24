<!--
	One bar for the whole file; each segment is drawn at its byte position, filled as far as
	it is downloaded. Drawn in SVG so no inline style attributes are needed (strict CSP).
-->
<script lang="ts">
	import type { SegmentView } from '#lib/api/bindings.ts';

	let { segments, total }: { segments: SegmentView[]; total: number } = $props();

	const W = 1000;
	let x = (offset: number) => (total > 0 ? (offset / total) * W : 0);
</script>

<svg
	viewBox="0 0 {W} 10"
	preserveAspectRatio="none"
	class="h-3 w-full overflow-hidden rounded-full fill-none bg-(--track)"
	role="img"
	aria-label="{segments.length} segments"
>
	{#each segments as s, i (s.start)}
		{@const width = x(s.end + 1) - x(s.start)}
		<rect
			data-testid="segment"
			x={x(s.start)}
			y="0"
			width={Math.max(0, x(s.start + s.downloaded) - x(s.start))}
			height="10"
			class={i % 2 === 0 ? 'fill-teal-500' : 'fill-cyan-500'}
		/>
		<line x1={x(s.start) + width} y1="0" x2={x(s.start) + width} y2="10" class="stroke-white/60 dark:stroke-zinc-950/60" vector-effect="non-scaling-stroke" />
	{/each}
</svg>
