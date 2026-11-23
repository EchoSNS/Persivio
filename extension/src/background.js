// Service worker: intercepts browser downloads and the context menu, sends them to the
// Persivio bridge on 127.0.0.1, and falls back to a normal browser download if Persivio
// is not reachable.

import {
	bridgeInit,
	bridgeUrl,
	describeStatus,
	downloadPayload,
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

function fallback(url, opts, reason) {
	if (!opts.fallback) {
		notify(`Not downloaded: ${reason}`);
		return;
	}
	bypass.add(url);
	chrome.downloads.download({ url }).catch(() => {});
	notify(`Downloading in the browser instead. ${reason}`);
}

chrome.downloads.onCreated.addListener(async (item) => {
	const opts = await options();
	if (!shouldIntercept(item, opts, bypass)) return;
	const url = item.finalUrl || item.url;
	// Stop the browser's copy first so the file is not downloaded twice.
	try {
		await chrome.downloads.cancel(item.id);
		await chrome.downloads.erase({ id: item.id });
	} catch {
		return; // Already finished or gone: leave it to the browser.
	}
	const error = await sendToPersivio({ url, referrer: item.referrer, filename: item.filename }, opts);
	if (error) fallback(url, opts, error);
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
