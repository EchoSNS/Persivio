import { describe, expect, it } from 'vitest';
import { formatBytes, formatEta, formatSpeed, percent } from './format.ts';

describe('formatBytes', () => {
	it('uses binary units with sensible precision', () => {
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(1023)).toBe('1023 B');
		expect(formatBytes(1536)).toBe('1.5 KiB');
		expect(formatBytes(150 * 1024 * 1024)).toBe('150 MiB');
		expect(formatBytes(4.7 * 1024 ** 3)).toBe('4.7 GiB');
	});
	it('handles invalid input', () => {
		expect(formatBytes(-1)).toBe('—');
		expect(formatBytes(Number.NaN)).toBe('—');
	});
});

describe('formatSpeed', () => {
	it('appends per second', () => {
		expect(formatSpeed(2.5 * 1024 * 1024)).toBe('2.5 MiB/s');
	});
});

describe('formatEta', () => {
	it('picks the two largest units', () => {
		expect(formatEta(12)).toBe('12s');
		expect(formatEta(249)).toBe('4m 09s');
		expect(formatEta(3900)).toBe('1h 05m');
		expect(formatEta(null)).toBe('—');
	});
});

describe('percent', () => {
	it('is clamped and null when the total is unknown', () => {
		expect(percent(50, 200)).toBe(25);
		expect(percent(300, 200)).toBe(100);
		expect(percent(0, 0)).toBe(100);
		expect(percent(5, null)).toBeNull();
	});
});
