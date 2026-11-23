<script lang="ts">
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { FolderOpen, Monitor, Moon, Plus, Sun, X } from '@lucide/svelte';
	import { commands, errorMessage, type SettingsView } from '#lib/api/index.ts';
	import { SHORTCUTS } from '#lib/shortcuts.ts';
	import { theme, type ThemeChoice } from '#lib/stores/theme.svelte.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';

	let { onsaved }: { onsaved: (s: SettingsView) => void } = $props();

	let saved = $state<SettingsView | null>(null);
	let draft = $state<SettingsView | null>(null);
	let error = $state<string | null>(null);
	let saving = $state(false);
	let newCategory = $state('');
	// Unit-friendly mirrors of byte values.
	let limitMiB = $state('');
	let segmentMiB = $state('');
	let seedRatio = $state('');

	let dirty = $derived(
		draft !== null &&
			saved !== null &&
			(JSON.stringify(toSettings(draft)) !== JSON.stringify(saved))
	);

	$effect(() => {
		commands.getSettings().then(reset, (e) => (error = errorMessage(e)));
	});

	function reset(s: SettingsView) {
		saved = s;
		draft = structuredClone(s);
		limitMiB = s.globalSpeedLimitBps === null ? '' : String(+(s.globalSpeedLimitBps / 1048576).toFixed(2));
		segmentMiB = String(+(s.minSegmentSize / 1048576).toFixed(2));
		seedRatio =
			s.torrent.seedRatioLimitPercent === null ? '' : String(s.torrent.seedRatioLimitPercent / 100);
		error = null;
	}

	/** The draft with the unit fields applied. */
	function toSettings(d: SettingsView): SettingsView {
		const limit = limitMiB.trim() === '' ? null : Math.round(Number(limitMiB) * 1048576);
		const ratio = seedRatio.trim() === '' ? null : Math.round(Number(seedRatio) * 100);
		return {
			...d,
			downloadDir: d.downloadDir?.trim() ? d.downloadDir.trim() : null,
			globalSpeedLimitBps: limit,
			minSegmentSize: Math.round(Number(segmentMiB) * 1048576),
			torrent: { ...d.torrent, seedRatioLimitPercent: ratio }
		};
	}

	async function save(e: SubmitEvent) {
		e.preventDefault();
		if (!draft) return;
		const next = toSettings(draft);
		for (const [name, v] of [
			['Global limit', next.globalSpeedLimitBps],
			['Minimum segment size', next.minSegmentSize],
			['Seed ratio', next.torrent.seedRatioLimitPercent]
		] as const) {
			if (v !== null && (!Number.isFinite(v) || v < 0)) {
				error = `${name} must be a positive number.`;
				return;
			}
		}
		saving = true;
		try {
			await commands.setSettings(next);
			reset(next);
			onsaved(next);
			toasts.info('Settings saved');
		} catch (err) {
			error = errorMessage(err);
		} finally {
			saving = false;
		}
	}

	async function browse() {
		if (!draft) return;
		const dir = await openDialog({ directory: true, defaultPath: draft.downloadDir ?? undefined });
		if (typeof dir === 'string') draft.downloadDir = dir;
	}

	function addCategory() {
		const name = newCategory.trim();
		if (!draft || name === '') return;
		if (draft.categories.some((c) => c.toLowerCase() === name.toLowerCase())) {
			error = `“${name}” already exists.`;
			return;
		}
		draft.categories = [...draft.categories, name];
		newCategory = '';
		error = null;
	}

	const THEMES: { value: ThemeChoice; label: string; Icon: typeof Sun }[] = [
		{ value: 'system', label: 'System', Icon: Monitor },
		{ value: 'light', label: 'Light', Icon: Sun },
		{ value: 'dark', label: 'Dark', Icon: Moon }
	];
</script>

{#snippet section(title: string)}
	<h2 class="mb-3 text-sm font-semibold">{title}</h2>
{/snippet}

<div class="h-full overflow-y-auto">
	<form class="mx-auto max-w-2xl space-y-6 px-6 py-6" onsubmit={save}>
		<div class="flex items-center justify-between">
			<h1 class="text-lg font-semibold">Settings</h1>
			<div class="flex gap-2">
				<button type="button" class="btn" disabled={!dirty || saving} onclick={() => saved && reset(saved)}>
					Discard changes
				</button>
				<button type="submit" class="btn-primary" disabled={!dirty || saving}>
					{saving ? 'Saving…' : 'Save'}
				</button>
			</div>
		</div>

		{#if error}
			<p role="alert" class="rounded-md bg-red-50 p-2.5 text-sm text-red-800 dark:bg-red-950 dark:text-red-200">{error}</p>
		{/if}

		{#if draft}
			<section class="panel p-4">
				{@render section('General')}
				<div class="space-y-3">
					<div>
						<label class="label" for="s-dir">Default download folder</label>
						<div class="flex gap-2">
							<input id="s-dir" class="input" placeholder="System Downloads folder" bind:value={draft.downloadDir} />
							<button type="button" class="btn shrink-0" onclick={browse}><FolderOpen size={14} /> Browse</button>
						</div>
					</div>
					<div class="grid gap-3 sm:grid-cols-2">
						<div>
							<label class="label" for="s-max">Downloads at the same time</label>
							<input id="s-max" class="input" type="number" min="1" max="256" bind:value={draft.maxConcurrentTasks} />
						</div>
					</div>
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" class="accent-teal-600" bind:checked={draft.autoResumeOnStartup} />
						Resume interrupted downloads when the app starts
					</label>
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Connections')}
				<div class="grid gap-3 sm:grid-cols-2">
					<div>
						<label class="label" for="s-conn">Connections per download</label>
						<input id="s-conn" class="input" type="number" min="1" max="32" bind:value={draft.defaultConnections} />
					</div>
					<div>
						<label class="label" for="s-seg">Minimum segment size (MiB)</label>
						<input id="s-seg" class="input" inputmode="decimal" bind:value={segmentMiB} />
					</div>
					<div>
						<label class="label" for="s-retries">Automatic retries</label>
						<input id="s-retries" class="input" type="number" min="0" bind:value={draft.maxRetries} />
					</div>
					<div>
						<label class="label" for="s-idle">Idle timeout (seconds)</label>
						<input id="s-idle" class="input" type="number" min="1" bind:value={draft.idleTimeoutSecs} />
					</div>
					<div>
						<label class="label" for="s-connect">Connect timeout (seconds)</label>
						<input id="s-connect" class="input" type="number" min="1" bind:value={draft.connectTimeoutSecs} />
					</div>
					<div class="sm:col-span-2">
						<label class="label" for="s-ua">User-Agent</label>
						<input id="s-ua" class="input font-mono text-xs" bind:value={draft.userAgent} />
					</div>
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Speed limit')}
				<label class="label" for="s-limit">Global download limit (MiB/s)</label>
				<input id="s-limit" class="input sm:w-48" inputmode="decimal" placeholder="Unlimited" bind:value={limitMiB} />
				<p class="hint">Applies to all HTTP downloads; torrents pick it up after a restart.</p>
			</section>

			<section class="panel p-4">
				{@render section('BitTorrent')}
				<div class="space-y-3">
					<div class="grid gap-3 sm:grid-cols-2">
						<div>
							<label class="label" for="s-port">Listening port</label>
							<input id="s-port" class="input" type="number" min="1" max="65535" placeholder="Automatic" bind:value={draft.torrent.listenPort} />
						</div>
						<div>
							<label class="label" for="s-ratio">Stop seeding at ratio</label>
							<input id="s-ratio" class="input" inputmode="decimal" placeholder="No limit" bind:value={seedRatio} disabled={draft.torrent.neverSeed} />
						</div>
						<div>
							<label class="label" for="s-seedtime">Stop seeding after (minutes)</label>
							<input id="s-seedtime" class="input" type="number" min="1" placeholder="No limit" bind:value={draft.torrent.seedTimeLimitMins} disabled={draft.torrent.neverSeed} />
						</div>
					</div>
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" class="accent-teal-600" bind:checked={draft.torrent.neverSeed} />
						Never seed (stop as soon as the download finishes)
					</label>
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" class="accent-teal-600" bind:checked={draft.torrent.dht} />
						Find peers through the DHT
					</label>
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" class="accent-teal-600" bind:checked={draft.torrent.upnp} />
						Ask the router to open the port (UPnP)
					</label>
					<p class="hint">Port, DHT and UPnP changes take effect after restarting the app.</p>
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Categories')}
				<ul class="mb-3 flex flex-wrap gap-2">
					{#each draft.categories as c (c)}
						<li class="inline-flex items-center gap-1 rounded-md bg-zinc-100 py-0.5 pr-0.5 pl-2 text-xs dark:bg-zinc-800">
							{c}
							<button
								type="button"
								class="icon-btn size-5"
								aria-label="Remove category {c}"
								onclick={() => draft && (draft.categories = draft.categories.filter((x) => x !== c))}
							>
								<X size={12} />
							</button>
						</li>
					{/each}
				</ul>
				<div class="flex gap-2 sm:w-80">
					<input
						class="input"
						placeholder="New category"
						maxlength="40"
						aria-label="New category name"
						bind:value={newCategory}
						onkeydown={(e) => {
							if (e.key === 'Enter') {
								e.preventDefault();
								addCategory();
							}
						}}
					/>
					<button type="button" class="btn shrink-0" onclick={addCategory}><Plus size={14} /> Add</button>
				</div>
			</section>
		{/if}

		<section class="panel p-4">
			{@render section('Appearance')}
			<div class="inline-flex rounded-md border border-zinc-300 p-0.5 dark:border-zinc-700" role="radiogroup" aria-label="Theme">
				{#each THEMES as t (t.value)}
					<button
						type="button"
						role="radio"
						aria-checked={theme.choice === t.value}
						class="inline-flex items-center gap-1.5 rounded px-3 py-1 text-xs {theme.choice === t.value
							? 'bg-teal-600 text-white'
							: 'text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800'}"
						onclick={() => theme.set(t.value)}
					>
						<t.Icon size={13} /> {t.label}
					</button>
				{/each}
			</div>
			<p class="hint">Saved on this computer immediately.</p>
		</section>

		<section class="panel p-4">
			{@render section('Keyboard shortcuts')}
			<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
				{#each SHORTCUTS as [keys, what] (keys)}
					<dt><span class="kbd">{keys}</span></dt>
					<dd class="text-zinc-600 dark:text-zinc-400">{what}</dd>
				{/each}
			</dl>
		</section>
	</form>
</div>
