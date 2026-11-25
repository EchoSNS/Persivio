// Pure logic of the extension (no browser APIs), tested with `node --test`.

/** Default options. The token is empty until the user pastes it from Persivio. */
export const DEFAULTS = Object.freeze({
	enabled: true,
	port: 47321,
	token: '',
	/** Take over downloads at least this large (MB). 0 = any size. */
	minSizeMb: 1,
	/** Always take over these file types, whatever their size. */
	extensions: ['zip', '7z', 'rar', 'iso', 'exe', 'msi', 'dmg', 'mp4', 'mkv', 'avi', 'mp3', 'pdf', 'tar', 'gz', 'xz', 'torrent'],
	/** Also open a browser download when Persivio cannot be reached. */
	fallback: true
});

/** Merges stored options over the defaults, ignoring unknown or invalid values. */
export function normalizeOptions(stored) {
	const o = { ...DEFAULTS, ...(stored ?? {}) };
	const port = Number(o.port);
	return {
		enabled: Boolean(o.enabled),
		port: Number.isInteger(port) && port >= 1024 && port <= 65535 ? port : DEFAULTS.port,
		token: typeof o.token === 'string' ? o.token.trim() : '',
		minSizeMb: Number.isFinite(Number(o.minSizeMb)) && Number(o.minSizeMb) >= 0 ? Number(o.minSizeMb) : DEFAULTS.minSizeMb,
		extensions: Array.isArray(o.extensions)
			? o.extensions.map((e) => String(e).trim().replace(/^\./, '').toLowerCase()).filter(Boolean)
			: [...DEFAULTS.extensions],
		fallback: Boolean(o.fallback)
	};
}

/** `https://a/b/file.tar.gz?x` → `gz`; `C:\x\a.ISO` → `iso`. */
export function extensionOf(nameOrUrl) {
	if (!nameOrUrl) return '';
	let path = String(nameOrUrl);
	try {
		path = new URL(path).pathname;
	} catch {
		// Not a URL: a file name or path.
	}
	const base = path.split(/[\\/]/).pop() ?? '';
	const dot = base.lastIndexOf('.');
	return dot > 0 ? base.slice(dot + 1).toLowerCase() : '';
}

export function isHttpUrl(url) {
	try {
		const { protocol } = new URL(url);
		return protocol === 'http:' || protocol === 'https:';
	} catch {
		return false;
	}
}

/**
 * Whether a browser download should go to Persivio instead.
 * `item` is a `downloads.DownloadItem`-like object; `bypass` holds URLs that must stay in
 * the browser (our own fallback re-downloads).
 */
export function shouldIntercept(item, options, bypass = new Set()) {
	if (!options.enabled || !options.token) return false;
	// Private windows use another cookie store and should leave no trace in the app.
	if (item.incognito) return false;
	const url = item.finalUrl || item.url;
	if (!isHttpUrl(url) || bypass.has(url) || bypass.has(item.url)) return false;
	if (item.state && item.state !== 'in_progress') return false;
	const ext = extensionOf(item.filename) || extensionOf(url);
	if (ext && options.extensions.includes(ext)) return true;
	const size = Number(item.totalBytes ?? item.fileSize ?? -1);
	return size > 0 && size >= options.minSizeMb * 1024 * 1024;
}

/** `Cookie` header value from `cookies.getAll` results (invalid entries skipped). */
export function cookieHeader(cookies) {
	return (cookies ?? [])
		.filter((c) => c && typeof c.name === 'string' && typeof c.value === 'string')
		.filter((c) => !/[;\r\n\0]/.test(c.name + c.value) && c.name.length > 0)
		.map((c) => `${c.name}=${c.value}`)
		.join('; ');
}

/** The last part of a path (browsers report full paths for `filename`). */
export function baseName(path) {
	return String(path ?? '').split(/[\\/]/).pop() ?? '';
}

/** Body for `POST /v1/offers`; `size` and `mime` are shown in Persivio's question. */
export function downloadPayload({ url, referrer, filename, size, mime }, cookies, userAgent) {
	const body = { url };
	if (referrer && isHttpUrl(referrer)) body.referrer = referrer;
	if (userAgent) body.userAgent = userAgent;
	const cookie = cookieHeader(cookies);
	if (cookie) body.cookies = cookie;
	const name = baseName(filename);
	if (name) body.filename = name;
	if (Number.isSafeInteger(size) && size > 0) body.size = size;
	if (mime) body.mime = String(mime);
	return body;
}

/** The same body for Persivio before 0.7, which knows neither `/v1/offers` nor the details. */
export function legacyPayload({ size: _size, mime: _mime, ...body }) {
	return body;
}

export function bridgeUrl(options, path) {
	return `http://127.0.0.1:${options.port}${path}`;
}

/** Request options for the bridge (token in the Authorization header). */
export function bridgeInit(options, method, body) {
	const init = {
		method,
		headers: { Authorization: `Bearer ${options.token}` },
		signal: AbortSignal.timeout(4000)
	};
	if (body !== undefined) {
		init.headers['Content-Type'] = 'application/json';
		init.body = JSON.stringify(body);
	}
	return init;
}

/** A user-facing explanation for a bridge HTTP status. */
export function describeStatus(status) {
	switch (status) {
		case 201:
		case 200:
		case 202:
			return 'ok';
		case 401:
			return 'Wrong pairing token: copy it again from Persivio → Settings → Browser integration.';
		case 403:
			return 'Persivio refused the request (not from this extension).';
		case 413:
			return 'Request too large.';
		case 429:
			return 'Too many downloads are waiting for your answer in Persivio.';
		case 422:
			return 'Persivio could not use this link.';
		default:
			return `Persivio answered with HTTP ${status}.`;
	}
}

/**
 * Offers a download to Persivio while the browser waits for its file name, so before the
 * browser asks where to save it (Chromium). Once Persivio has it, the browser's copy is
 * cancelled and its question never appears. Otherwise the browser goes on as if nothing
 * had happened: 'kept'. Or, without `keepInBrowser`, it is cancelled too: 'dropped'.
 */
export async function takeOver(id, api, send, keepInBrowser) {
	let error;
	try {
		error = await send();
	} catch (e) {
		error = String(e);
	}
	if (error !== null && keepInBrowser) return { result: 'kept', error };
	await api.cancel(id).catch(() => {});
	await api.erase(id).catch(() => {});
	return { result: error === null ? 'sent' : 'dropped', error };
}

/**
 * For browsers without that event (Firefox).
 * Moves a running browser download to Persivio without losing it: pause, send, and only
 * after Persivio accepted it cancel the browser's copy. On failure the same download
 * resumes (requesting it again would break one-time links).
 *
 * `api` is { pause, resume, cancel, erase } taking a download id; `send` resolves to an
 * error message or null. The result is 'sent', 'kept' (with `error`) or 'skipped'.
 */
export async function handOff(id, api, send, keepInBrowser) {
	try {
		await api.pause(id);
	} catch {
		return { result: 'skipped', error: null }; // already finished: the browser has it
	}
	let error;
	try {
		error = await send();
	} catch (e) {
		error = String(e);
	}
	if (error === null) {
		await api.cancel(id).catch(() => {});
		await api.erase(id).catch(() => {});
		return { result: 'sent', error: null };
	}
	if (keepInBrowser) await api.resume(id).catch(() => {});
	else await api.cancel(id).catch(() => {});
	return { result: 'kept', error };
}
