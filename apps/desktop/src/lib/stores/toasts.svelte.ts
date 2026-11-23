// Short-lived notifications in the corner. Errors stay longer than infos.

export type ToastAction = { label: string; run: () => void };

export type Toast = {
	id: number;
	kind: 'info' | 'warn' | 'error';
	message: string;
	action?: ToastAction;
};

class Toasts {
	items = $state<Toast[]>([]);
	#next = 1;

	#push(kind: Toast['kind'], message: string, ms: number, action?: ToastAction): void {
		const id = this.#next++;
		// Keep the newest five; identical messages in a row are merged.
		if (this.items.at(-1)?.message === message) return;
		this.items = [...this.items.slice(-4), { id, kind, message, ...(action ? { action } : {}) }];
		setTimeout(() => this.dismiss(id), ms);
	}

	info(message: string): void {
		this.#push('info', message, 4000);
	}
	warn(message: string): void {
		this.#push('warn', message, 8000);
	}
	error(message: string): void {
		this.#push('error', message, 10000);
	}
	/** A non-intrusive offer with one action button (e.g. "Add" for copied links). */
	offer(message: string, action: ToastAction): void {
		this.#push('info', message, 15000, action);
	}
	dismiss(id: number): void {
		this.items = this.items.filter((t) => t.id !== id);
	}
}

export const toasts = new Toasts();
