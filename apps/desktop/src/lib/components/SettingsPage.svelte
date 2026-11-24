<script lang="ts">
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { Copy, FileText, FolderOpen, Monitor, Moon, Plus, RefreshCw, Save, Sun, X } from '@lucide/svelte';
	import {
		commands,
		errorMessage,
		type BridgeStatus,
		type SettingsView,
		type UpdateView
	} from '#lib/api/index.ts';
	import Modal from './Modal.svelte';
	import { SHORTCUTS } from '#lib/shortcuts.ts';
	import { theme, type ThemeChoice } from '#lib/stores/theme.svelte.ts';
	import { toasts } from '#lib/stores/toasts.svelte.ts';

	type Props = {
		onsaved: (s: SettingsView) => void;
		onupdate: (u: UpdateView) => void;
		onshownotes: () => void;
	};
	let { onsaved, onupdate, onshownotes }: Props = $props();

	let bridge = $state<BridgeStatus | null>(null);
	let version = $state('');
	let checking = $state(false);

	$effect(() => {
		commands.appVersion().then((v) => (version = v));
		refreshBridge();
	});

	function refreshBridge() {
		commands.bridgeStatus().then(
			(s) => (bridge = s),
			() => {}
		);
	}

	async function copyToken() {
		if (!draft) return;
		try {
			await navigator.clipboard.writeText(draft.desktop.bridgeToken);
			toasts.info('Token copied');
		} catch {
			toasts.warn('Copy failed: select the token and copy it manually.');
		}
	}

	async function newToken() {
		if (!draft) return;
		try {
			draft.desktop.bridgeToken = await commands.newBridgeToken();
			toasts.info('New token: save, then paste it into the extension.');
		} catch (e) {
			error = errorMessage(e);
		}
	}

	async function checkNow() {
		checking = true;
		try {
			const found = await commands.checkForUpdate();
			if (found) onupdate(found);
			else toasts.info('Persivio is up to date.');
		} catch (e) {
			toasts.warn(errorMessage(e));
		} finally {
			checking = false;
		}
	}

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
			const problems = await commands.setSettings(next);
			// Reload: saving may have created the bridge token.
			const fresh = await commands.getSettings();
			reset(fresh);
			onsaved(fresh);
			setTimeout(refreshBridge, 300);
			toasts.info('Settings saved');
			for (const problem of problems) toasts.warn(problem);
		} catch (err) {
			error = errorMessage(err);
		} finally {
			saving = false;
		}
	}

	let report = $state<string | null>(null);

	async function showReport() {
		try {
			report = await commands.diagnosticsReport();
		} catch (e) {
			toasts.error(errorMessage(e));
		}
	}

	async function saveReport() {
		try {
			if (await commands.saveDiagnostics()) toasts.info('Report saved');
		} catch (e) {
			toasts.error(errorMessage(e));
		}
	}

	async function openLogs() {
		try {
			await commands.openLogsFolder();
		} catch (e) {
			toasts.error(errorMessage(e));
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

	type Desktop = SettingsView['desktop'];
	/** Desktop settings that are on/off switches. */
	type DesktopSwitch = { [K in keyof Desktop]: Desktop[K] extends boolean ? K : never }[keyof Desktop];
	const DESKTOP_TOGGLES: { key: DesktopSwitch; label: string; hint?: string }[] = [
		{
			key: 'closeToTray',
			label: 'Keep running in the system tray when the window is closed',
			hint: 'Quit from the tray icon menu.'
		},
		{ key: 'notifyOnComplete', label: 'Notify when a download completes' },
		{ key: 'notifyOnFailure', label: 'Notify when a download fails' },
		{
			key: 'preventSleep',
			label: 'Keep the computer awake while downloading',
			hint: 'The screen may still turn off; only sleep is prevented.'
		},
		{
			key: 'watchClipboard',
			label: 'Offer to add links I copy',
			hint: 'Checks copied text once a second for http(s) and magnet links. Nothing is added without your click, and the text is never stored.'
		},
		{
			key: 'autostart',
			label: 'Start Persivio when I sign in',
			hint: 'Starts minimized to the tray.'
		},
		{
			key: 'handleMagnetLinks',
			label: 'Open magnet links with Persivio',
			hint: 'Windows and Linux. On macOS, choose Persivio as the magnet handler in another torrent app or the system. Turning this off only removes Persivio\x27s own registration.'
		}
	];

	const THEMES: { value: ThemeChoice; label: string; Icon: typeof Sun }[] = [
		{ value: 'system', label: 'System', Icon: Monitor },
		{ value: 'light', label: 'Light', Icon: Sun },
		{ value: 'dark', label: 'Dark', Icon: Moon }
	];
</script>

<Modal open={report !== null} title="Diagnostics report" wide onclose={() => (report = null)}>
	<pre class="max-h-[60vh] overflow-auto rounded-xl border border-(--line) bg-(--field) p-3 text-xs whitespace-pre-wrap select-text">{report}</pre>
	{#snippet footer()}
		<button type="button" class="btn" onclick={saveReport}><Save size={14} /> Save report…</button>
	{/snippet}
</Modal>

{#snippet section(title: string)}
	<h2 class="mb-3 text-sm font-semibold">{title}</h2>
{/snippet}

<div class="h-full overflow-y-auto">
	<form class="mx-auto max-w-2xl space-y-4 px-2 pb-6" onsubmit={save}>
		<div class="glass sticky top-0 z-10 flex items-center justify-between rounded-full py-2 pr-2.5 pl-5">
			<h1 class="text-base font-semibold">Settings</h1>
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
			<p role="alert" class="rounded-xl bg-red-500/15 p-2.5 text-sm text-red-800 dark:text-red-200">{error}</p>
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
				<p class="hint">Shared by all downloads. Changes apply immediately, also to running ones.</p>
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
					<p class="hint">Port, DHT and UPnP changes take effect once no torrent has run for a minute, or after a restart.</p>
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Desktop')}
				<div class="space-y-2.5">
					{#each DESKTOP_TOGGLES as t (t.key)}
						<label class="flex items-start gap-2 text-sm">
							<input type="checkbox" class="mt-0.5 accent-teal-600" bind:checked={draft.desktop[t.key]} />
							<span>
								{t.label}
								{#if t.hint}<span class="hint block">{t.hint}</span>{/if}
							</span>
						</label>
					{/each}
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Categories')}
				<ul class="mb-3 flex flex-wrap gap-2">
					{#each draft.categories as c (c)}
						<li class="chip pr-0.5">
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

			<section class="panel p-4">
				{@render section('Browser integration')}
				<div class="space-y-3">
					<label class="flex items-start gap-2 text-sm">
						<input type="checkbox" class="mt-0.5 accent-teal-600" bind:checked={draft.desktop.browserBridge} />
						<span>
							Accept downloads from the Persivio browser extension
							<span class="hint block">
								Listens on this computer only (127.0.0.1). The extension must send the pairing token below.
							</span>
						</span>
					</label>
					<div class="grid gap-3 sm:grid-cols-[8rem_1fr]">
						<div>
							<label class="label" for="s-bport">Port</label>
							<input id="s-bport" class="input" type="number" min="1024" max="65535" bind:value={draft.desktop.bridgePort} />
						</div>
						<div>
							<label class="label" for="s-btoken">Pairing token</label>
							<div class="flex gap-2">
								<input
									id="s-btoken"
									class="input font-mono text-xs"
									readonly
									placeholder="Created when you save with the bridge on"
									value={draft.desktop.bridgeToken}
								/>
								<button type="button" class="btn shrink-0" disabled={!draft.desktop.bridgeToken} onclick={copyToken}>
									<Copy size={14} /> Copy
								</button>
								<button type="button" class="btn shrink-0" onclick={newToken} title="The extension must be paired again">
									<RefreshCw size={14} /> New
								</button>
							</div>
						</div>
					</div>
					<p class="text-xs {bridge?.error ? 'text-red-700 dark:text-red-300' : 'text-(--muted)'}">
						{bridge?.running
							? `Running on 127.0.0.1:${saved?.desktop.bridgePort}.`
							: bridge?.error
								? bridge.error
								: 'Not running.'}
					</p>
					<p class="hint">
						Install the extension from the <code>extension</code> folder (see its README), then paste the
						token in its options.
					</p>
				</div>
			</section>

			<section class="panel p-4">
				{@render section('Updates')}
				<div class="space-y-3">
					<p class="text-sm">Persivio {version}</p>
					<label class="flex items-center gap-2 text-sm">
						<input type="checkbox" class="accent-teal-600" bind:checked={draft.desktop.checkForUpdates} />
						Check for updates when Persivio starts
					</label>
					<div class="flex gap-2">
						<button type="button" class="btn" disabled={checking} onclick={checkNow}>
							<RefreshCw size={14} /> {checking ? 'Checking…' : 'Check now'}
						</button>
						<button type="button" class="btn" onclick={onshownotes}>What's new</button>
					</div>
				</div>
			</section>
		{/if}

		<section class="panel p-4">
			{@render section('Diagnostics')}
			<p class="text-sm">
				A report for reporting a problem: version, system, settings and the log. Links, file names,
				addresses, folders and tokens are removed. Nothing is sent anywhere.
			</p>
			<div class="mt-3 flex flex-wrap gap-2">
				<button type="button" class="btn" onclick={showReport}><FileText size={14} /> View report</button>
				<button type="button" class="btn" onclick={saveReport}><Save size={14} /> Save report…</button>
				<button type="button" class="btn" onclick={openLogs}><FolderOpen size={14} /> Open logs folder</button>
			</div>
		</section>

		<section class="panel p-4">
			{@render section('Appearance')}
			<div class="inline-flex rounded-full border border-(--line) bg-(--control) p-0.5" role="radiogroup" aria-label="Theme">
				{#each THEMES as t (t.value)}
					<button
						type="button"
						role="radio"
						aria-checked={theme.choice === t.value}
						class="inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs focus-visible:ring-2 focus-visible:ring-(--ring) focus-visible:outline-none {theme.choice === t.value
							? 'bg-teal-700 text-white'
							: 'hover:bg-(--control-hover)'}"
						onclick={() => theme.set(t.value)}
					>
						<t.Icon size={13} /> {t.label}
					</button>
				{/each}
			</div>
			<label class="mt-3 flex items-start gap-2 text-sm">
				<input
					type="checkbox"
					class="mt-0.5 accent-teal-600"
					checked={theme.surface === 'glass'}
					onchange={(e) => theme.setSurface(e.currentTarget.checked ? 'glass' : 'solid')}
				/>
				<span>
					Glass effects
					<span class="hint block">
						Translucent, blurred surfaces. Turn off for solid ones, which is faster on older graphics
						hardware. Always off when the system asks for less transparency or more contrast.
					</span>
				</span>
			</label>
			<p class="hint">Saved on this computer immediately.</p>
		</section>

		<section class="panel p-4">
			{@render section('Keyboard shortcuts')}
			<dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-xs">
				{#each SHORTCUTS as [keys, what] (keys)}
					<dt><span class="kbd">{keys}</span></dt>
					<dd class="text-(--muted)">{what}</dd>
				{/each}
			</dl>
		</section>
	</form>
</div>
