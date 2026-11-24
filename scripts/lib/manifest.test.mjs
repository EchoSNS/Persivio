import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildManifest, publicKeyId, signatureInfo } from './manifest.mjs';

const b64 = (x) => Buffer.from(x).toString('base64');
const raw = (id, size) => b64(Buffer.concat([Buffer.from('Ed'), Buffer.from(id, 'hex'), Buffer.alloc(size)]));
const key = (id) => b64(`untrusted comment: minisign public key\n${raw(id, 32)}\n`);
const sig = (id, version) =>
	b64(`untrusted comment: signature from tauri secret key\n${raw(id, 64)}\ntrusted comment: timestamp:1\tfile:x${version ? `\tversion:${version}` : ''}\n${raw(id, 62)}\n`);

const ID = '0011223344556677';
const FILES = [
	'Persivio_1.2.3_x64-setup.exe',
	'Persivio_1.2.3_x64_en-US.msi',
	'Persivio_1.2.3_universal.app.tar.gz',
	'Persivio_1.2.3_universal.dmg',
	'Persivio_1.2.3_amd64.AppImage',
	'Persivio_1.2.3_amd64.deb'
];
const input = (over = {}) => ({
	version: '1.2.3',
	notes: 'notes',
	date: '2026-11-25T00:00:00Z',
	baseUrl: 'https://example.com/v1.2.3',
	files: FILES,
	signatures: Object.fromEntries(FILES.map((f) => [f, sig(ID, '1.2.3')])),
	pubkey: key(ID),
	...over
});

describe('manifest', () => {
	it('reads key ids and the signed version', () => {
		assert.equal(publicKeyId(key(ID)), ID);
		assert.deepEqual(signatureInfo(sig(ID, '1.2.3')), { keyId: ID, version: '1.2.3' });
		assert.equal(signatureInfo(sig(ID, null)).version, null);
	});

	it('serves every platform from the matching package', () => {
		const m = buildManifest(input());
		assert.equal(m.version, '1.2.3');
		const url = (k) => m.platforms[k].url.split('/').pop();
		assert.equal(url('windows-x86_64'), FILES[0]);
		assert.equal(url('windows-x86_64-msi'), FILES[1]);
		assert.equal(url('darwin-aarch64'), FILES[2]);
		assert.equal(url('darwin-x86_64'), FILES[2]);
		assert.equal(url('linux-x86_64'), FILES[4]);
		assert.equal(url('linux-x86_64-deb'), FILES[5]);
		assert.ok(m.platforms['linux-x86_64'].url.startsWith('https://example.com/v1.2.3/'));
	});

	it('refuses anything that installed copies would refuse', () => {
		const other = { ...input().signatures, [FILES[0]]: sig('ffffffffffffffff', '1.2.3') };
		assert.throws(() => buildManifest(input({ signatures: other })), /another key/);
		const old = { ...input().signatures, [FILES[0]]: sig(ID, '1.2.2') };
		assert.throws(() => buildManifest(input({ signatures: old })), /signed for version 1\.2\.2/);
		assert.throws(() => buildManifest(input({ signatures: {} })), /has no signature/);
		assert.throws(() => buildManifest(input({ files: FILES.slice(1) })), /no update package for windows-x86_64/);
		assert.throws(() => buildManifest(input({ version: '9.0.0' })), /is not version 9\.0\.0/);
	});
});
