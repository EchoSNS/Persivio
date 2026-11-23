// The task list as reactive state (Svelte 5 runes), kept in sync with the engine.
//
// Sync protocol: listen to events first, then load the list, then apply the events
// that arrived meanwhile. Afterwards events and progress batches update the list in
// place; a `resync` event (the app missed events) reloads it.
//
// `tasks` is `$state.raw`: rows are replaced, never mutated, so Svelte only compares
// references instead of deeply proxying 500+ rows (Phase 8: smooth scrolling).

import { Channel } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
	commands,
	errorMessage,
	events,
	isCommandError,
	type AddRequest,
	type EngineUiEvent,
	type MoveDirectionView,
	type ProgressView
} from '#lib/api/index.ts';
import { recordSpeeds } from './history.ts';
import { applyEvent, applyProgress, fromList, type TaskRow } from './reducer.ts';
import { toasts } from './toasts.svelte.ts';

export class TaskStore {
	tasks = $state.raw<TaskRow[]>([]);
	loaded = $state(false);
	/** Set when the engine could not start (e.g. the CLI is using the data folder). */
	unavailable = $state<string | null>(null);
	/** Bumped on every progress batch so views of `speeds()` update. */
	historyVersion = $state(0);

	#history = new Map<string, number[]>();
	#unlisten: UnlistenFn | null = null;
	#buffer: EngineUiEvent[] | null = [];

	async connect(): Promise<void> {
		this.#unlisten = await events.engineUiEvent.listen((e) => this.#onEvent(e.payload));
		await this.reload();
		const channel = new Channel<ProgressView[]>();
		channel.onmessage = (batch) => {
			this.tasks = applyProgress(this.tasks, batch);
			recordSpeeds(this.#history, batch);
			this.historyVersion += 1;
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
			else toasts.error(errorMessage(e));
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
		if (event.type === 'task_removed') this.#history.delete(event.id);
		if (result.warning) toasts.warn(result.warning);
		if (result.resync) {
			this.#buffer = [];
			void this.reload();
		}
	}

	/** Recent download speeds of a task (reactive through `historyVersion`). */
	speeds(id: string): readonly number[] {
		void this.historyVersion;
		return this.#history.get(id) ?? [];
	}

	/** Runs a command; shows its error as a toast. Returns whether it worked. */
	async #run(action: () => Promise<unknown>): Promise<boolean> {
		try {
			await action();
			return true;
		} catch (e) {
			toasts.error(errorMessage(e));
			return false;
		}
	}

	/** Adds a download; returns its id, or `null` (the error is shown). */
	async add(request: AddRequest): Promise<string | null> {
		try {
			return await commands.addTask(request);
		} catch (e) {
			toasts.error(errorMessage(e));
			return null;
		}
	}
	pause(id: string) {
		return this.#run(() => commands.pauseTask(id));
	}
	resume(id: string) {
		return this.#run(() => commands.resumeTask(id));
	}
	cancel(id: string) {
		return this.#run(() => commands.cancelTask(id));
	}
	remove(id: string, deleteFiles: boolean) {
		return this.#run(() => commands.removeTask(id, deleteFiles));
	}
	reveal(id: string) {
		return this.#run(() => commands.revealTask(id));
	}
	move(id: string, direction: MoveDirectionView) {
		return this.#run(() => commands.moveTask(id, direction));
	}
	selectFiles(id: string, files: number[] | null) {
		return this.#run(() => commands.selectFiles(id, files));
	}
	pauseAll() {
		return this.#run(() => commands.pauseAll());
	}
	resumeAll() {
		return this.#run(() => commands.resumeAll());
	}
}
