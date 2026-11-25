import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
	DEFAULTS,
	baseName,
	bridgeInit,
	bridgeUrl,
	cookieHeader,
	describeStatus,
	downloadPayload,
	extensionOf,
	handOff,
	legacyPayload,
	takeOver,
	normalizeOptions,
	shouldIntercept
} from '../src/lib.js';

const paired = normalizeOptions({ token: 'a'.repeat(64) });

describe('normalizeOptions', () => {
	it('fills defaults and rejects invalid values', () => {
		const o = normalizeOptions({ port: 80, minSizeMb: -1, extensions: ['.ZIP', ' iso ', ''], token: ' t ' });
		assert.equal(o.port, DEFAULTS.port);
		assert.equal(o.minSizeMb, DEFAULTS.minSizeMb);
		assert.deepEqual(o.extensions, ['zip', 'iso']);
		assert.equal(o.token, 't');
		assert.equal(normalizeOptions(undefined).enabled, true);
	});
});

describe('shouldIntercept', () => {
	const item = (over) => ({ url: 'https://a.example/file.bin', filename: '', totalBytes: -1, state: 'in_progress', ...over });

	it('takes over listed types and large files, only when paired and enabled', () => {
		assert.equal(shouldIntercept(item({ url: 'https://a.example/ubuntu.iso' }), paired), true);
		assert.equal(shouldIntercept(item({ filename: 'C:\\Downloads\\setup.EXE' }), paired), true);
		assert.equal(shouldIntercept(item({ totalBytes: 5 * 1024 * 1024 }), paired), true);
		assert.equal(shouldIntercept(item({ totalBytes: 1000 }), paired), false, 'small and unlisted');
		assert.equal(shouldIntercept(item({ totalBytes: -1 }), paired), false, 'unknown size, unlisted');
		assert.equal(shouldIntercept(item({ url: 'https://a.example/x.iso' }), normalizeOptions({})), false, 'not paired');
		assert.equal(shouldIntercept(item({ url: 'https://a.example/x.iso' }), { ...paired, enabled: false }), false);
	});

	it('never takes blob:/data: downloads, finished ones, or our own fallbacks', () => {
		assert.equal(shouldIntercept(item({ url: 'blob:https://a.example/123', filename: 'x.zip' }), paired), false);
		assert.equal(shouldIntercept(item({ url: 'data:application/zip;base64,AA', filename: 'x.zip' }), paired), false);
		assert.equal(shouldIntercept(item({ url: 'https://a.example/x.iso', state: 'complete' }), paired), false);
		assert.equal(shouldIntercept(item({ url: 'https://a.example/x.iso', incognito: true }), paired), false);
		const url = 'https://a.example/x.iso';
		assert.equal(shouldIntercept(item({ url }), paired, new Set([url])), false);
	});
});

describe('payload', () => {
	it('builds the cookie header and skips broken cookies', () => {
		assert.equal(
			cookieHeader([
				{ name: 'session', value: 'abc' },
				{ name: 'bad', value: 'x\r\nInjected: 1' },
				{ name: '', value: 'v' },
				{ name: 'theme', value: 'dark' }
			]),
			'session=abc; theme=dark'
		);
		assert.equal(cookieHeader(undefined), '');
	});

	it('sends only what is present and safe', () => {
		assert.deepEqual(
			downloadPayload(
				{ url: 'https://a.example/f.iso', referrer: 'https://a.example/page', filename: '/home/me/Downloads/f.iso' },
				[{ name: 's', value: '1' }],
				'Mozilla/5.0'
			),
			{ url: 'https://a.example/f.iso', referrer: 'https://a.example/page', userAgent: 'Mozilla/5.0', cookies: 's=1', filename: 'f.iso' }
		);
		assert.deepEqual(downloadPayload({ url: 'https://a.example/f', referrer: 'javascript:x' }, [], ''), {
			url: 'https://a.example/f'
		});
	});

	it('addresses only the loopback bridge with the token header', () => {
		assert.equal(bridgeUrl({ port: 47321 }, '/v1/ping'), 'http://127.0.0.1:47321/v1/ping');
		const init = bridgeInit({ token: 'tok' }, 'POST', { url: 'https://a/b' });
		assert.equal(init.headers.Authorization, 'Bearer tok');
		assert.equal(init.headers['Content-Type'], 'application/json');
		assert.equal(init.body, '{"url":"https://a/b"}');
		assert.equal(bridgeInit({ token: 'tok' }, 'GET').body, undefined);
	});
});

describe('helpers', () => {
	it('extract extensions and base names', () => {
		assert.equal(extensionOf('https://a/b/archive.tar.gz?x=1'), 'gz');
		assert.equal(extensionOf('C:\\x\\A.ISO'), 'iso');
		assert.equal(extensionOf('https://a/b/noext'), '');
		assert.equal(extensionOf('.bashrc'), '');
		assert.equal(baseName('C:\\Users\\me\\f.zip'), 'f.zip');
		assert.equal(describeStatus(401).includes('token'), true);
		assert.equal(describeStatus(201), 'ok');
	});
});

describe('handOff', () => {
	function api(over = {}) {
		const calls = [];
		const step = (name) => async () => void calls.push(name);
		return { calls, pause: step('pause'), resume: step('resume'), cancel: step('cancel'), erase: step('erase'), ...over };
	}

	it('cancels the browser copy only after Persivio accepted it', async () => {
		const a = api();
		const send = async () => {
			a.calls.push('send');
			return null;
		};
		assert.equal((await handOff(1, a, send, true)).result, 'sent');
		assert.deepEqual(a.calls, ['pause', 'send', 'cancel', 'erase']);
	});

	it('resumes the same download when Persivio refuses or throws', async () => {
		const throwing = async () => {
			throw new Error('boom');
		};
		for (const send of [async () => 'not running', throwing]) {
			const a = api();
			const r = await handOff(1, a, send, true);
			assert.equal(r.result, 'kept');
			assert.ok(r.error);
			assert.deepEqual(a.calls, ['pause', 'resume']);
		}
	});

	it('respects "no fallback" and leaves finished downloads alone', async () => {
		const a = api();
		await handOff(1, a, async () => 'not running', false);
		assert.deepEqual(a.calls, ['pause', 'cancel']);
		const done = api({
			pause: async () => {
				throw new Error('not in progress');
			}
		});
		assert.equal((await handOff(1, done, async () => null, true)).result, 'skipped');
		assert.deepEqual(done.calls, []);
	});
});

describe('takeOver', () => {
	const api = () => {
		const calls = [];
		const f = (name) => async (id) => void calls.push(`${name} ${id}`);
		return { calls, pause: f('pause'), resume: f('resume'), cancel: f('cancel'), erase: f('erase') };
	};

	it('cancels the browser download only once Persivio has it', async () => {
		const a = api();
		const send = async () => {
			assert.deepEqual(a.calls, [], 'untouched until Persivio answers');
			return null;
		};
		assert.deepEqual(await takeOver(7, a, send, true), { result: 'sent', error: null });
		assert.deepEqual(a.calls, ['cancel 7', 'erase 7']);
	});

	it('leaves the download alone when Persivio cannot take it', async () => {
		for (const send of [async () => 'not running', async () => Promise.reject(new Error('boom'))]) {
			const a = api();
			const r = await takeOver(7, a, send, true);
			assert.equal(r.result, 'kept');
			assert.ok(r.error);
			assert.deepEqual(a.calls, []);
		}
	});

	it('without the fallback nothing is downloaded, and it says so', async () => {
		const a = api();
		assert.deepEqual(await takeOver(7, a, async () => 'not running', false), { result: 'dropped', error: 'not running' });
		assert.deepEqual(a.calls, ['cancel 7', 'erase 7']);
	});
});

describe('details for the question', () => {
	it('sends size and type when the browser knows them', () => {
		const item = { url: 'https://a.example/f', filename: 'f.zip', size: 5, mime: 'application/zip' };
		const body = downloadPayload(item, [], '');
		assert.deepEqual(body, { url: 'https://a.example/f', filename: 'f.zip', size: 5, mime: 'application/zip' });
		assert.deepEqual(legacyPayload(body), { url: 'https://a.example/f', filename: 'f.zip' });
		for (const size of [-1, 0, 1.5, undefined, NaN]) {
			assert.ok(!('size' in downloadPayload({ url: 'https://a.example/f', size }, [], '')), String(size));
		}
	});
});
