<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import TaskItem from '#lib/components/TaskItem.svelte';
	import { TaskStore } from '#lib/stores/tasks.svelte.ts';

	const store = new TaskStore();
	let input = $state('');
	let adding = $state(false);

	onMount(() => {
		void store.connect();
	});
	onDestroy(() => store.disconnect());

	async function add(event: SubmitEvent) {
		event.preventDefault();
		const sources = input
			.split(/\r?\n/)
			.map((s) => s.trim())
			.filter((s) => s.length > 0);
		if (sources.length === 0) return;
		adding = true;
		let allAdded = true;
		for (const source of sources) {
			const added = await store.add({
				source,
				destDir: null,
				fileName: null,
				connections: null,
				speedLimitBps: null,
				category: null
			});
			allAdded &&= added;
		}
		adding = false;
		if (allAdded) input = '';
	}

	function remove(id: string, name: string) {
		const deleteFiles = confirm(
			`Also delete the downloaded files of "${name}"?\n\nOK deletes the files. Cancel keeps them.`
		);
		void store.remove(id, deleteFiles);
	}
</script>

<div class="flex h-full flex-col bg-zinc-50 text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
	<header class="border-b border-zinc-200 bg-white px-4 py-3 dark:border-zinc-800 dark:bg-zinc-900">
		<form class="flex gap-2" onsubmit={add}>
			<label class="sr-only" for="source">URL or magnet link</label>
			<input
				id="source"
				class="min-w-0 flex-1 rounded-md border border-zinc-300 bg-transparent px-3 py-2 text-sm outline-none focus:border-teal-500 focus:ring-2 focus:ring-teal-500/30 dark:border-zinc-700"
				placeholder="Paste a URL or magnet link"
				autocomplete="off"
				spellcheck="false"
				bind:value={input}
				disabled={store.unavailable !== null}
			/>
			<button
				class="rounded-md bg-teal-600 px-4 py-2 text-sm font-medium text-white hover:bg-teal-700 focus-visible:ring-2 focus-visible:ring-teal-500/50 focus-visible:outline-none disabled:opacity-50"
				disabled={adding || input.trim() === '' || store.unavailable !== null}
			>
				Add
			</button>
		</form>
		<div class="mt-2 flex gap-2">
			<button class="btn" onclick={() => store.pauseAll()}>Pause all</button>
			<button class="btn" onclick={() => store.resumeAll()}>Resume all</button>
		</div>
	</header>

	{#if store.unavailable}
		<div
			role="alert"
			class="m-4 rounded-md border border-red-300 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200"
		>
			<p class="font-medium">The download engine is not running.</p>
			<p>{store.unavailable}</p>
			<p class="mt-1 text-xs">
				Close any running <code>persivio</code> command-line session, then restart the app.
			</p>
		</div>
	{/if}

	{#if store.notice}
		<div
			role="status"
			class="mx-4 mt-3 flex items-start gap-3 rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200"
		>
			<p class="flex-1">{store.notice}</p>
			<button class="text-xs underline" onclick={() => store.dismiss()}>Dismiss</button>
		</div>
	{/if}

	<main class="flex-1 overflow-y-auto p-4">
		{#if store.loaded && store.tasks.length === 0 && !store.unavailable}
			<p class="mt-16 text-center text-sm text-zinc-500">
				No downloads yet. Paste a link above to start.
			</p>
		{:else}
			<ul class="flex flex-col gap-2">
				{#each store.tasks as task (task.id)}
					<TaskItem
						{task}
						onpause={() => store.pause(task.id)}
						onresume={() => store.resume(task.id)}
						oncancel={() => store.cancel(task.id)}
						onremove={() => remove(task.id, task.name)}
						onreveal={() => store.reveal(task.id)}
					/>
				{/each}
			</ul>
		{/if}
	</main>
</div>
