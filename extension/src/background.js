// Service worker: intercepts browser downloads and the context menu, offers them to the
// Persivio bridge on 127.0.0.1 (Persivio asks the user), and falls back to a normal browser download if Persivio
// is not reachable.

import {
	bridgeInit,
	bridgeUrl,
	describeStatus,
	downloadPayload,
	handOff,
	isHttpUrl,
	legacyPayload,
	normalizeOptions,
	shouldIntercept,
	takeOver
} from './lib.js';

const MENU_ID = 'persivio-download-link';
/** URLs we re-download in the browser after a failed hand-off (never intercept them again). */
const bypass = new Set();

async function options() {
	return normalizeOptions(await chrome.storage.local.get(null));
}

function notify(message) {
	chrome.notifications.create({
		type: 'basic',
		iconUrl: chrome.runtime.getURL('icons/icon-128.png'),
		title: 'Persivio',
		message
	});
}

/** Sends one download to Persivio. Returns an error message, or null on success. */
async function sendToPersivio(download, opts) {
	let cookies = [];
	try {
		cookies = await chrome.cookies.getAll({ url: download.url });
	} catch {
		// No cookie access for this URL: send without (public downloads still work).
	}
	const body = downloadPayload(download, cookies, navigator.userAgent);
	try {
		let response = await fetch(bridgeUrl(opts, '/v1/offers'), bridgeInit(opts, 'POST', body));
		if (response.status === 404) {
			response = await fetch(bridgeUrl(opts, '/v1/downloads'), bridgeInit(opts, 'POST', legacyPayload(body)));
		}
		const message = describeStatus(response.status);
		return message === 'ok' ? null : message;
	} catch {
		return 'Persivio is not running, or browser integration is off in its settings.';
	}
}

/** Opens a link in the browser's own downloader (context menu only). */
function fallback(url, opts, reason) {
	if (!opts.fallback) {
		notify(`Not downloaded: ${reason}`);
		return;
	}
	bypass.add(url);
	chrome.downloads.download({ url }).catch(() => {});
	notify(`Downloading in the browser instead. ${reason}`);
}

const downloadsApi = {
	pause: (id) => chrome.downloads.pause(id),
	resume: (id) => chrome.downloads.resume(id),
	cancel: (id) => chrome.downloads.cancel(id),
	erase: (id) => chrome.downloads.erase({ id })
};

/** What Persivio needs to know about a browser download. */
function details(item) {
	return {
		url: item.finalUrl || item.url,
		referrer: item.referrer,
		filename: item.filename,
		size: item.totalBytes,
		mime: item.mime
	};
}

/** Whether a download is ours to take. Uses up a bypass. */
function wanted(item, opts) {
	return !bypass.delete(item.url) && shouldIntercept(item, opts);
}

function explain(result, error) {
	if (result === 'kept') notify(`Downloading in the browser instead. ${error}`);
	if (result === 'dropped') notify(`Not downloaded: ${error}`);
}

if (chrome.downloads.onDeterminingFilename) {
	// Chromium: the browser waits for `suggest` before it asks where to save the file.
	chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
		(async () => {
			const opts = await options();
			if (!wanted(item, opts)) return 'kept';
			const { result, error } = await takeOver(
				item.id,
				downloadsApi,
				() => sendToPersivio(details(item), opts),
				opts.fallback
			);
			explain(result, error);
			return result;
		})()
			.catch(() => 'kept')
			// Nothing to suggest for a cancelled download; Chromium has forgotten it.
			.then((result) => result === 'kept' && suggest());
		return true; // `suggest` is called later
	});
} else {
	chrome.downloads.onCreated.addListener(async (item) => {
		const opts = await options();
		if (!wanted(item, opts)) return;
		const send = () => sendToPersivio(details(item), opts);
		const { result, error } = await handOff(item.id, downloadsApi, send, opts.fallback);
		if (result === 'kept') explain(opts.fallback ? 'kept' : 'dropped', error);
	});
}

chrome.runtime.onInstalled.addListener(() => {
	chrome.contextMenus.create({
		id: MENU_ID,
		title: 'Download with Persivio',
		contexts: ['link']
	});
});

chrome.contextMenus.onClicked.addListener(async (info) => {
	if (info.menuItemId !== MENU_ID || !isHttpUrl(info.linkUrl)) return;
	const opts = await options();
	if (!opts.token) {
		notify('Pair the extension first: paste the token from Persivio in the extension options.');
		chrome.runtime.openOptionsPage();
		return;
	}
	const error = await sendToPersivio({ url: info.linkUrl, referrer: info.pageUrl }, opts);
	if (error) fallback(info.linkUrl, opts, error);
});

// The options page asks the worker to test the connection (same code path as downloads).
chrome.runtime.onMessage.addListener((message, _sender, reply) => {
	if (message?.type !== 'ping') return false;
	(async () => {
		const opts = await options();
		try {
			const response = await fetch(bridgeUrl(opts, '/v1/ping'), bridgeInit(opts, 'GET'));
			const status = describeStatus(response.status);
			if (status !== 'ok') return reply({ ok: false, message: status });
			const info = await response.json();
			reply({ ok: true, message: `Connected to Persivio ${info.version}.` });
		} catch {
			reply({ ok: false, message: 'Persivio is not reachable on this port. Is the app running with browser integration on?' });
		}
	})();
	return true; // reply asynchronously
});
