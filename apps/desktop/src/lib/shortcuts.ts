// Keyboard shortcuts for the task list. Ctrl+V is handled through the `paste` event
// (that needs no clipboard permission); everything else maps here.

export type Action =
	| 'add'
	| 'select-all'
	| 'toggle-pause'
	| 'remove'
	| 'open-details'
	| 'up'
	| 'down'
	| 'extend-up'
	| 'extend-down'
	| 'first'
	| 'last'
	| 'escape'
	| 'settings'
	| 'search';

export type KeyLike = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>;

/** The action for a key press, or `null`. Inside text fields only Escape counts. */
export function actionFor(e: KeyLike, inTextField: boolean): Action | null {
	const mod = e.ctrlKey || e.metaKey;
	if (e.key === 'Escape') return 'escape';
	if (inTextField || e.altKey) return null;
	if (mod) {
		switch (e.key.toLowerCase()) {
			case 'a':
				return 'select-all';
			case 'n':
				return 'add';
			case ',':
				return 'settings';
			case 'f':
				return 'search';
			default:
				return null;
		}
	}
	switch (e.key) {
		case ' ':
			return 'toggle-pause';
		case 'Delete':
			return 'remove';
		case 'Enter':
			return 'open-details';
		case 'ArrowUp':
			return e.shiftKey ? 'extend-up' : 'up';
		case 'ArrowDown':
			return e.shiftKey ? 'extend-down' : 'down';
		case 'Home':
			return 'first';
		case 'End':
			return 'last';
		default:
			return null;
	}
}

/** Whether keyboard focus is in a place where typing means text, not shortcuts. */
export function isTextField(target: EventTarget | null): boolean {
	if (!(target instanceof HTMLElement)) return false;
	if (target.isContentEditable) return true;
	if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
	// In an open, styled dropdown the focus is on the option.
	if (target instanceof HTMLOptionElement) return true;
	if (target instanceof HTMLInputElement) {
		return !['checkbox', 'radio', 'button', 'submit', 'range'].includes(target.type);
	}
	return false;
}

/** Shown in the help line and the settings page. */
export const SHORTCUTS: readonly [string, string][] = [
	['Ctrl+V', 'Add pasted links'],
	['Ctrl+N', 'New download'],
	['Space', 'Pause / resume selected'],
	['Delete', 'Remove selected'],
	['Ctrl+A', 'Select all'],
	['↑ ↓ Home End', 'Move (Shift extends)'],
	['Enter', 'Details'],
	['Ctrl+F', 'Search'],
	['Ctrl+,', 'Settings'],
	['Esc', 'Close / clear selection']
];
