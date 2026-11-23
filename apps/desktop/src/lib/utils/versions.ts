// Which release notes to show after an update.

import type { ReleaseNotesView } from '#lib/api/bindings.ts';

/** `1.2.3` → [1, 2, 3]; pre-release tags are ignored. */
export function versionKey(v: string): [number, number, number] {
	const [a = 0, b = 0, c = 0] = (v.split(/[-+]/)[0] ?? '')
		.split('.')
		.map((n) => Number.parseInt(n, 10) || 0);
	return [a, b, c];
}

export function compareVersions(a: string, b: string): number {
	const [x, y] = [versionKey(a), versionKey(b)];
	for (let i = 0; i < 3; i++) {
		const d = (x[i] ?? 0) - (y[i] ?? 0);
		if (d !== 0) return d;
	}
	return 0;
}

/**
 * Notes to show when `current` runs and `lastSeen` was shown before: every release
 * newer than `lastSeen` up to `current`. First run (no `lastSeen`): just `current`.
 * Nothing when the version did not change.
 */
export function notesToShow(
	all: ReleaseNotesView[],
	current: string,
	lastSeen: string | null
): ReleaseNotesView[] {
	if (lastSeen !== null && compareVersions(current, lastSeen) <= 0) return [];
	return all.filter(
		(r) =>
			compareVersions(r.version, current) <= 0 &&
			(lastSeen === null ? compareVersions(r.version, current) === 0 : compareVersions(r.version, lastSeen) > 0)
	);
}

const KEY = 'persivio.lastSeenVersion';

export function loadLastSeen(): string | null {
	try {
		return localStorage.getItem(KEY);
	} catch {
		return null;
	}
}

export function saveLastSeen(version: string): void {
	try {
		localStorage.setItem(KEY, version);
	} catch {
		// Shown again next time; harmless.
	}
}
