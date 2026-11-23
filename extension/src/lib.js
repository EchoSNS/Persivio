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

/** Body for `POST /v1/downloads`. */
export function downloadPayload({ url, referrer, filename }, cookies, userAgent) {
	const body = { url };
	if (referrer && isHttpUrl(referrer)) body.referrer = referrer;
	if (userAgent) body.userAgent = userAgent;
	const cookie = cookieHeader(cookies);
	if (cookie) body.cookies = cookie;
	const name = baseName(filename);
	if (name) body.filename = name;
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
			return 'ok';
		case 401:
			return 'Wrong pairing token: copy it again from Persivio → Settings → Browser integration.';
		case 403:
			return 'Persivio refused the request (not from this extension).';
		case 413:
			return 'Request too large.';
		case 422:
			return 'Persivio could not use this link.';
		default:
			return `Persivio answered with HTTP ${status}.`;
	}
}
