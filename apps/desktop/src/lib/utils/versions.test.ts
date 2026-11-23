import { describe, expect, it } from 'vitest';
import type { ReleaseNotesView } from '#lib/api/bindings.ts';
import { compareVersions, notesToShow, versionKey } from './versions.ts';

const release = (version: string): ReleaseNotesView => ({
	version,
	date: '2026-11-23',
	sections: [{ title: 'Added', items: [`in ${version}`] }]
});
const all = ['0.4.0', '0.3.0', '0.2.0', '0.1.0'].map(release);
const versions = (r: ReleaseNotesView[]) => r.map((x) => x.version);

describe('versions', () => {
	it('compare numerically', () => {
		expect(versionKey('1.2.3-beta')).toEqual([1, 2, 3]);
		expect(compareVersions('0.10.0', '0.9.0')).toBeGreaterThan(0);
		expect(compareVersions('1.0.0', '1.0')).toBe(0);
	});

	it('show every release since the last one seen, up to the running one', () => {
		expect(versions(notesToShow(all, '0.4.0', '0.2.0'))).toEqual(['0.4.0', '0.3.0']);
		expect(versions(notesToShow(all, '0.3.0', '0.2.0'))).toEqual(['0.3.0']);
	});

	it('show only the current release on first run, and nothing without a change', () => {
		expect(versions(notesToShow(all, '0.2.0', null))).toEqual(['0.2.0']);
		expect(notesToShow(all, '0.3.0', '0.3.0')).toEqual([]);
		expect(notesToShow(all, '0.2.0', '0.3.0')).toEqual([]); // downgrade: nothing
	});
});
