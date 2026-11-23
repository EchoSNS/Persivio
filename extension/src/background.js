// Service worker: intercepts browser downloads and the context menu, sends them to the
// Persivio bridge on 127.0.0.1, and falls back to a normal browser download if Persivio
// is not reachable.

import {
	bridgeInit,
	bridgeUrl,
	describeStatus,
	downloadPayload,
	handOff,
	isHttpUrl,
	normalizeOptions,
	shouldIntercept
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
		const response = await fetch(bridgeUrl(opts, '/v1/downloads'), bridgeInit(opts, 'POST', body));
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

chrome.downloads.onCreated.addListener(async (item) => {
	const opts = await options();
	if (bypass.delete(item.url) || !shouldIntercept(item, opts)) return;
	const url = item.finalUrl || item.url;
	const { result, error } = await handOff(
		item.id,
		downloadsApi,
		() => sendToPersivio({ url, referrer: item.referrer, filename: item.filename }, opts),
		opts.fallback
	);
	if (result !== 'kept') return;
	notify(opts.fallback ? `Downloading in the browser instead. ${error}` : `Not downloaded: ${error}`);
});

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
