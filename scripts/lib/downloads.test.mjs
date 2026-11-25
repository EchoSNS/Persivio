import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { describeDownloads, withDownloads } from './downloads.mjs';

const assets = [
	'latest.json',
	'Persivio-extension_1.2.3.zip',
	'Persivio_1.2.3_amd64.AppImage',
	'Persivio_1.2.3_amd64.AppImage.sig',
	'Persivio_1.2.3_amd64.deb',
	'Persivio_1.2.3_universal.app.tar.gz',
	'Persivio_1.2.3_universal.dmg',
	'Persivio_1.2.3_x64-setup.exe',
	'Persivio_1.2.3_x64-setup.exe.sig',
	'Persivio_1.2.3_x64_en-US.msi',
	'SHA256SUMS.txt'
].map((name) => ({ name, size: 8 * 1024 * 1024 }));
const baseUrl = 'https://example.com/v1.2.3';

describe('describeDownloads', () => {
	const text = describeDownloads({ assets, baseUrl });
	const rows = text.split('\n').filter((l) => l.startsWith('| ') && l.includes(baseUrl));

	it('links every installer once, the recommended one first', () => {
		assert.equal(rows.length, 6);
		assert.match(rows[0], /\[Persivio_1\.2\.3_x64-setup\.exe\]\(https:\/\/example\.com\/v1\.2\.3\/Persivio_1\.2\.3_x64-setup\.exe\) \(8 MiB\)/);
		assert.match(rows[0], /Recommended/);
	});

	it('keeps files that are not for people out of the table, but explains them', () => {
		assert.ok(!rows.some((r) => /\.sig|latest\.json|tar\.gz|SHA256SUMS/.test(r)));
		for (const name of ['latest.json', '….sig', 'app.tar.gz', 'SHA256SUMS.txt']) assert.ok(text.includes(name), name);
	});

	it('refuses a file nobody described', () => {
		assert.throws(
			() => describeDownloads({ assets: [...assets, { name: 'surprise.rpm', size: 1 }], baseUrl }),
			/surprise\.rpm/
		);
	});
});

describe('withDownloads', () => {
	it('adds the part once, however often it runs', () => {
		const once = withDownloads('### Fixed\n- x\r\n', '## Which file do I need?\n\nA');
		assert.equal(once, '### Fixed\n- x\n\n## Which file do I need?\n\nA');
		assert.equal(withDownloads(once, '## Which file do I need?\n\nB'), '### Fixed\n- x\n\n## Which file do I need?\n\nB');
	});
});
