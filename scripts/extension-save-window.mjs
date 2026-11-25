// Does the browser open its own "save as" window next to Persivio's question? Windows only:
// the real browser with "ask where to save each file" on (throw-away profile), the real
// bridge, and an extension that was asleep, as it usually is. Opens a browser window.
//
//   node scripts/extension-save-window.mjs [path to brave.exe or another Chromium]
//
// Uses the DevTools protocol directly: Playwright replaces the browser's download handling.

import { execSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const EXT = path.join(ROOT, 'extension');
const BROWSER = process.argv[2] ?? 'C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe';
const DEBUG_PORT = 9444;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const out = execSync('cargo test -p persivio-desktop --test bridge --no-run --message-format=json', { cwd: ROOT, stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 1e9 }).toString();
const exe = out.split('\n').filter((l) => l.startsWith('{')).map((l) => JSON.parse(l)).filter((m) => m.executable && m.target?.name === 'bridge').pop().executable;

/** Prints every save window of this site for six seconds (it lives in its own process). */
const WATCH = `param([string]$Site)
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class V { public delegate bool E(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(E p, IntPtr l);
[DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
[DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
public static void Look(string site){ EnumWindows((h,l)=>{ if(IsWindowVisible(h)){ var c=new StringBuilder(99);GetClassName(h,c,99); var n=new StringBuilder(199);GetWindowText(h,n,199); if(c.ToString()=="#32770" && n.ToString().Contains(site)) Console.WriteLine("WINDOW "+n);} return true;},IntPtr.Zero);} }
"@
"READY"
$t = [Diagnostics.Stopwatch]::StartNew()
while ($t.ElapsedMilliseconds -lt 6000) { [V]::Look($Site); Start-Sleep -Milliseconds 15 }`;

/** One download; returns what could be seen. */
async function run({ paired, running }) {
	const server = spawn(exe, ['serve_for_a_real_browser', '--ignored', '--nocapture', '--exact'], { stdio: ['pipe', 'pipe', 'inherit'], env: { ...process.env, PERSIVIO_TEST_ASK: '1' } });
	let log = '';
	const info = await new Promise((resolve) => {
		server.stdout.on('data', (d) => {
			log += d;
			const m = log.match(/SERVING (\{.*\})/);
			if (m) resolve(JSON.parse(m[1]));
		});
	});
	const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'persivio-browser-'));
	fs.mkdirSync(path.join(profile, 'Default'));
	fs.writeFileSync(path.join(profile, 'Default', 'Preferences'), JSON.stringify({ download: { prompt_for_download: true, default_directory: profile } }));
	const browser = spawn(BROWSER, [`--user-data-dir=${profile}`, `--remote-debugging-port=${DEBUG_PORT}`, `--disable-extensions-except=${EXT}`, `--load-extension=${EXT}`, '--disable-features=DisableLoadExtensionCommandLineSwitch', '--no-first-run', '--no-default-browser-check', '--window-size=700,500', 'about:blank'], { stdio: 'ignore' });
	try {
		let version;
		for (let i = 0; i < 60 && !version; i++) {
			await sleep(500);
			version = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json/version`).then((r) => r.json()).catch(() => undefined);
		}
		const ws = new WebSocket(version.webSocketDebuggerUrl);
		await new Promise((r) => (ws.onopen = r));
		let seq = 0;
		const pending = new Map();
		ws.onmessage = (m) => {
			const d = JSON.parse(m.data);
			const p = pending.get(d.id);
			pending.delete(d.id);
			if (p) d.error ? p.reject(new Error(d.error.message)) : p.resolve(d.result);
		};
		const send = (method, params = {}, sessionId) =>
			new Promise((resolve, reject) => {
				pending.set(++seq, { resolve, reject });
				ws.send(JSON.stringify({ id: seq, method, params, sessionId }));
			});
		const run = (expression, sessionId) => send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: true }, sessionId);
		const open = async (url) => {
			const { targetId } = await send('Target.createTarget', { url });
			return { targetId, session: (await send('Target.attachToTarget', { targetId, flatten: true })).sessionId };
		};
		const workers = async () => (await send('Target.getTargets')).targetInfos.filter((t) => t.type === 'service_worker' && /^chrome-extension:.*background\.js/.test(t.url));
		for (let i = 0; i < 40 && (await workers()).length === 0; i++) await sleep(400);
		const [worker] = await workers();
		const { sessionId: sw } = await send('Target.attachToTarget', { targetId: worker.targetId, flatten: true });
		// Nothing listens on the other port.
		await run(`chrome.storage.local.set(${JSON.stringify({ port: running ? info.port : 47999, token: paired ? info.token : '' })})`, sw);
		await send('Target.detachFromTarget', { sessionId: sw });

		const page = await open(new URL(info.url).origin + '/file/none');
		await sleep(1200);
		const [name, ...value] = info.cookie.split('=');
		await send('Network.setCookie', { name, value: value.join('='), url: info.url }, page.session);

		const own = await open(new URL('/manifest.json', worker.url).href);
		await send('ServiceWorker.enable', {}, own.session);
		await send('ServiceWorker.stopAllWorkers', {}, own.session);
		await sleep(1500);
		if ((await workers()).length > 0) throw new Error('the extension is still awake');
		await send('Target.closeTarget', { targetId: own.targetId });

		const script = path.join(profile, 'watch.ps1');
		fs.writeFileSync(script, WATCH);
		const watcher = spawn('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, new URL(info.url).host]);
		let seen = '';
		watcher.stdout.on('data', (d) => (seen += d));
		while (!seen.includes('READY')) await sleep(50);
		await sleep(300);
		await run(`(() => { const a = document.createElement('a'); a.href = ${JSON.stringify(info.url)}; a.download = ''; document.body.append(a); a.click(); })()`, page.session);
		await new Promise((r) => watcher.on('exit', r));
		const offer = log.match(/OFFER (.*)/)?.[1];
		return { window: seen.includes('WINDOW'), offers: offer ? JSON.parse(offer) : [] };
	} finally {
		try {
			execSync(`taskkill /PID ${browser.pid} /T /F`, { stdio: 'ignore' });
		} catch {
			// Already gone.
		}
		server.kill();
		await sleep(1500);
		fs.rmSync(profile, { recursive: true, force: true, maxRetries: 8 });
	}
}

const check = (name, ok, extra = '') => {
	console.log(ok ? 'PASS' : 'FAIL', name, extra);
	if (!ok) process.exitCode = 1;
};
const control = await run({ paired: false, running: true });
check('control: without Persivio the browser asks where to save', control.window && control.offers.length === 0);
const taken = await run({ paired: true, running: true });
check('with Persivio the browser does not ask', !taken.window);
check('Persivio asks, with the details', taken.offers.length === 1 && taken.offers[0].fileName === 'private.zip' && taken.offers[0].size === 3000000 && taken.offers[0].sendsCredentials, JSON.stringify(taken.offers[0] ?? null));
const stopped = await run({ paired: true, running: false });
check('Persivio not running: the browser downloads as usual', stopped.window && stopped.offers.length === 0);
process.exit();
