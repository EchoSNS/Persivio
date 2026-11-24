<!-- An available update: its notes, then download → install → restart with progress. -->
<script lang="ts">
	import { Channel } from '@tauri-apps/api/core';
	import { commands, errorMessage, type UpdateProgress, type UpdateView } from '#lib/api/index.ts';
	import { formatBytes } from '#lib/utils/format.ts';
	import Modal from './Modal.svelte';

	type Props = { update: UpdateView | null; onclose: () => void };
	let { update, onclose }: Props = $props();

	let progress = $state<UpdateProgress | null>(null);
	let error = $state<string | null>(null);

	async function install() {
		error = null;
		const channel = new Channel<UpdateProgress>();
		channel.onmessage = (p) => (progress = p);
		progress = { type: 'downloading', downloaded: 0, total: null };
		try {
			// On success the app restarts; this promise only settles on failure.
			await commands.installUpdate(channel);
		} catch (e) {
			progress = null;
			error = errorMessage(e);
		}
	}
</script>

<Modal open={update !== null} title="Update available" onclose={() => progress === null && onclose()}>
	{#if update}
		<p class="text-sm">
			Persivio <strong>{update.version}</strong> is available (you have {update.currentVersion}).
		</p>
		{#if update.notes}
			<pre class="mt-3 max-h-60 overflow-y-auto rounded-xl border border-(--line) bg-(--field) p-3 text-xs whitespace-pre-wrap">{update.notes}</pre>
		{/if}
		<p class="hint mt-3">
			Installing stops running downloads safely; they continue after the restart.
		</p>
		{#if progress?.type === 'downloading'}
			<p class="mt-3 text-sm" aria-live="polite">
				Downloading… {formatBytes(progress.downloaded)}{progress.total ? ` of ${formatBytes(progress.total)}` : ''}
			</p>
		{:else if progress?.type === 'installing'}
			<p class="mt-3 text-sm" aria-live="polite">Installing… Persivio will restart.</p>
		{/if}
		{#if error}<p class="field-error">{error}</p>{/if}
	{/if}
	{#snippet footer()}
		<button class="btn" disabled={progress !== null} onclick={onclose}>Later</button>
		<button class="btn-primary" disabled={progress !== null} onclick={install}>Install and restart</button>
	{/snippet}
</Modal>
