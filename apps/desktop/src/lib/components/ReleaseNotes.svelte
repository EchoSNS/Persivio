<!-- Release notes as plain text lists (never rendered as HTML). -->
<script lang="ts">
	import type { ReleaseNotesView } from '#lib/api/bindings.ts';

	let { releases }: { releases: ReleaseNotesView[] } = $props();
</script>

<div class="space-y-5">
	{#each releases as r (r.version)}
		<section>
			<h3 class="text-sm font-semibold">
				Version {r.version} <span class="font-normal text-(--muted)">· {r.date}</span>
			</h3>
			{#each r.sections as s (s.title)}
				<h4 class="mt-2 text-xs font-semibold tracking-wide text-(--muted) uppercase">{s.title}</h4>
				<ul class="mt-1 list-disc space-y-1 pl-5 text-sm">
					{#each s.items as item, i (i)}<li>{item}</li>{/each}
				</ul>
			{/each}
		</section>
	{/each}
</div>
