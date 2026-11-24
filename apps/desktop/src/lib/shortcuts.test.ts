// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { actionFor, isTextField } from './shortcuts.ts';

describe('isTextField', () => {
	it('covers everything that uses keys itself, including the options of an open dropdown', () => {
		for (const tag of ['textarea', 'select', 'option', 'input']) {
			expect(isTextField(document.createElement(tag)), tag).toBe(true);
		}
		const box = document.createElement('input');
		box.type = 'checkbox';
		for (const el of [box, document.createElement('button'), document.body, null]) {
			expect(isTextField(el)).toBe(false);
		}
	});

	it('keeps arrow keys away from the task list there', () => {
		const key = { key: 'ArrowDown', ctrlKey: false, metaKey: false, shiftKey: false, altKey: false };
		expect(actionFor(key, true)).toBeNull();
		expect(actionFor(key, false)).toBe('down');
	});
});
