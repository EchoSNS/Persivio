// The real extension in a real Chromium browser (throw-away profile), against the real
// bridge and a test site with a login. Manual tests 9.1 to 9.5 and 9.9.
//
//   npm install --no-save playwright-core   (in any folder above this file)
//   node scripts/extension-e2e.mjs [path to brave.exe or another Chromium]

import { chromium } from 'playwright-core';
import { spawn, execSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const EXT = path.join(ROOT, 'extension');
const BROWSER = process.argv[2] ?? 'C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe';
// Find the test binary, then run the interactive test directly.
const out = execSync('cargo test -p persivio-desktop --test bridge --no-run --message-format=json', { cwd: ROOT, stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 1e9 }).toString();
const exe = out.split('\n').filter((l) => l.startsWith('{')).map((l) => JSON.parse(l)).filter((m) => m.executable && m.target?.name === 'bridge').pop().executable;
const proc = spawn(exe, ['serve_for_a_real_browser', '--ignored', '--nocapture', '--exact'], { stdio: ['pipe', 'pipe', 'inherit'] });
const info = await new Promise((resolve) => {
	let buf = '';
	proc.stdout.on('data', (d) => {
		buf += d;
		const m = buf.match(/SERVING (\{.*\})/);
		if (m) resolve(JSON.parse(m[1]));
	});
});
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'persivio-brave-'));
const ctx = await chromium.launchPersistentContext(profile, {
	executablePath: BROWSER,
	headless: false,
	args: ['--headless=new', `--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`, '--disable-features=DisableLoadExtensionCommandLineSwitch', '--no-first-run']
});
const check = (name, ok, extra = '') => {
	console.log(ok ? 'PASS' : 'FAIL', name, extra);
	if (!ok) process.exitCode = 1;
};
const until = async (f, ms = 15000) => {
	const end = Date.now() + ms;
	while (Date.now() < end) {
		const v = await f();
		if (v) return v;
		await new Promise((r) => setTimeout(r, 200));
	}
	return null;
};
try {
	const sw = ctx.serviceWorkers()[0] ?? (await ctx.waitForEvent('serviceworker', { timeout: 15000 }));
	const id = new URL(sw.url()).host;
	const opt = await ctx.newPage();
	await opt.goto(`chrome-extension://${id}/options.html`);
	const set = (o) => opt.evaluate((o) => chrome.storage.local.set(o), o);
	const ping = () => opt.evaluate(() => chrome.runtime.sendMessage({ type: 'ping' }));
	await set({ port: info.port, token: 'b'.repeat(64) });
	const wrong = await ping();
	check('9.2 wrong token is refused with a clear text', !wrong.ok && /token/i.test(wrong.message), wrong.message);
	await set({ token: info.token });
	const right = await ping();
	check('9.1 test connection', right.ok, right.message);
	await opt.reload();
	const btn = opt.getByRole('button', { name: /test/i });
	await btn.click();
	const shown = await until(async () => (await opt.locator('body').innerText()).match(/Connected to Persivio[^\n]*/)?.[0]);
	check('9.1 the button in the options page', !!shown, shown);

	// Log in: the site's cookie, set like a real login would.
	const url = new URL(info.url);
	const [name, ...value] = info.cookie.split('=');
	await ctx.addCookies([{ name, value: value.join('='), url: url.origin }]);
	const page = await ctx.newPage();
	await page.route('**/account', (r) => r.fulfill({ contentType: 'text/html', body: `<a id=d href="${info.url}" download>Download</a>` }));
	await page.goto(url.origin + '/account');
	await page.click('#d');
	const file = path.join(info.dir, 'private.zip');
	const done = await until(() => fs.existsSync(file) && fs.statSync(file).size === 3000000);
	const hash = done ? crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex') : '';
	check('9.5 a download behind a login arrives in Persivio, intact', hash === info.sha256);
	const left = await sw.evaluate(() => chrome.downloads.search({}));
	check('9.4 nothing is left in the browser', left.length === 0, JSON.stringify(left.map((d) => [d.state, d.filename])));

	const web = await ctx.newPage();
	const res = await web.goto(`http://127.0.0.1:${info.port}/v1/ping`);
	check('9.9 the address typed in a tab needs the token', res.status() === 401 || res.status() === 403, 'HTTP ' + res.status());
	await web.goto(url.origin + '/file/none');
	const evil = await web.evaluate(async ([port, token]) => {
		try {
			const r = await fetch(`http://127.0.0.1:${port}/v1/ping`, { headers: { Authorization: 'Bearer ' + token } });
			return 'HTTP ' + r.status;
		} catch (e) {
			return 'blocked: ' + e.message;
		}
	}, [info.port, info.token]);
	check('a web page with a stolen token is refused', !evil.startsWith('HTTP 2'), evil);

	// Persivio gone: the browser keeps its download.
	proc.stdin.end('\n');
	await new Promise((r) => proc.once('exit', r));
	const off = await ping();
	check('9.3 not running is explained', !off.ok && /not reachable/.test(off.message), off.message);
} finally {
	await ctx.close();
	if (proc.exitCode === null) proc.kill();
	fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
}
