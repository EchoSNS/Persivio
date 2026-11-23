// Serves dist-updates/ on http://127.0.0.1:8787 for the local update channel.
// Read-only, loopback only, no directory listing, no path traversal. See docs/updates.md.
//
//   node scripts/serve-updates.mjs [port]

import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('../dist-updates', import.meta.url)));
const port = Number(process.argv[2] ?? 8787);
const types = { '.json': 'application/json', '.exe': 'application/octet-stream', '.sig': 'text/plain' };

createServer(async (req, res) => {
	const send = (status, text) => {
		res.writeHead(status, { 'content-type': 'text/plain' });
		res.end(text);
	};
	if (req.method !== 'GET' && req.method !== 'HEAD') return send(405, 'method not allowed');
	let path;
	try {
		path = decodeURIComponent(new URL(req.url ?? '/', 'http://localhost').pathname);
	} catch {
		return send(400, 'bad request');
	}
	const file = resolve(join(root, path));
	if (!file.startsWith(root + sep)) return send(403, 'forbidden');
	try {
		const info = await stat(file);
		if (!info.isFile()) return send(404, 'not found');
		res.writeHead(200, {
			'content-type': types[extname(file)] ?? 'application/octet-stream',
			'content-length': info.size,
			'cache-control': 'no-store'
		});
		if (req.method === 'HEAD') return res.end();
		createReadStream(file).pipe(res);
		console.log(`${new Date().toISOString()} ${req.method} ${path}`);
	} catch {
		send(404, 'not found');
	}
}).listen(port, '127.0.0.1', () => {
	console.log(`Serving ${root} on http://127.0.0.1:${port}/ (Ctrl+C to stop)`);
});
