// The task list as reactive state (Svelte 5 runes), kept in sync with the engine.
//
// Sync protocol: listen to events first, then load the list, then apply the events
// that arrived meanwhile. Afterwards events and progress batches update the list in
// place; a `resync` event (the app missed events) reloads it.

import { Channel } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
	commands,
	errorMessage,
	events,
	isCommandError,
	type AddRequest,
	type EngineUiEvent,
	type ProgressView
} from '#lib/api/index.ts';
import { applyEvent, applyProgress, fromList, type TaskRow } from './reducer.ts';

export class TaskStore {
	tasks = $state<TaskRow[]>([]);
	loaded = $state(false);
	/** Last error or warning to show; cleared by `dismiss`. */
	notice = $state<string | null>(null);
	/** Set when the engine could not start (e.g. the CLI is using the data folder). */
	unavailable = $state<string | null>(null);

	#unlisten: UnlistenFn | null = null;
	#buffer: EngineUiEvent[] | null = [];

	async connect(): Promise<void> {
		this.#unlisten = await events.engineUiEvent.listen((e) => this.#onEvent(e.payload));
		await this.reload();
		const channel = new Channel<ProgressView[]>();
		channel.onmessage = (batch) => {
			this.tasks = applyProgress(this.tasks, batch);
		};
		await this.#run(() => commands.subscribeProgress(channel));
	}

	disconnect(): void {
		this.#unlisten?.();
		this.#unlisten = null;
	}

	async reload(): Promise<void> {
		try {
			const list = await commands.listTasks();
			let rows = fromList(list);
			for (const event of this.#buffer ?? []) rows = applyEvent(rows, event).rows;
			this.#buffer = null;
			this.tasks = rows;
			this.unavailable = null;
		} catch (e) {
			if (isCommandError(e) && e.kind === 'unavailable') this.unavailable = e.message;
			else this.notice = errorMessage(e);
		} finally {
			this.loaded = true;
		}
	}

	#onEvent(event: EngineUiEvent): void {
		if (this.#buffer) {
			this.#buffer.push(event);
			return;
		}
		const result = applyEvent(this.tasks, event);
		this.tasks = result.rows;
		if (result.warning) this.notice = result.warning;
		if (result.resync) {
			this.#buffer = [];
			void this.reload();
		}
	}

	dismiss(): void {
		this.notice = null;
	}

	/** Runs a command; shows its error instead of throwing. Returns whether it worked. */
	async #run(action: () => Promise<unknown>): Promise<boolean> {
		try {
			await action();
			return true;
		} catch (e) {
			this.notice = errorMessage(e);
			return false;
		}
	}

	add(request: AddRequest): Promise<boolean> {
		return this.#run(() => commands.addTask(request));
	}
	pause(id: string): Promise<boolean> {
		return this.#run(() => commands.pauseTask(id));
	}
	resume(id: string): Promise<boolean> {
		return this.#run(() => commands.resumeTask(id));
	}
	cancel(id: string): Promise<boolean> {
		return this.#run(() => commands.cancelTask(id));
	}
	remove(id: string, deleteFiles: boolean): Promise<boolean> {
		return this.#run(() => commands.removeTask(id, deleteFiles));
	}
	reveal(id: string): Promise<boolean> {
		return this.#run(() => commands.revealTask(id));
	}
	pauseAll(): Promise<boolean> {
		return this.#run(() => commands.pauseAll());
	}
	resumeAll(): Promise<boolean> {
		return this.#run(() => commands.resumeAll());
	}
}
