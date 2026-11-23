// Options page: stores settings in chrome.storage.local (the token never syncs).
import { DEFAULTS, normalizeOptions } from './src/lib.js';

const $ = (id) => document.getElementById(id);
const status = (text, ok = true) => {
	$('status').textContent = text;
	$('status').className = ok ? 'ok' : 'error';
};

async function load() {
	const o = normalizeOptions(await chrome.storage.local.get(null));
	$('enabled').checked = o.enabled;
	$('token').value = o.token;
	$('port').value = String(o.port);
	$('minSizeMb').value = String(o.minSizeMb);
	$('extensions').value = o.extensions.join(', ');
	$('fallback').checked = o.fallback;
}

$('form').addEventListener('submit', async (e) => {
	e.preventDefault();
	const o = normalizeOptions({
		enabled: $('enabled').checked,
		token: $('token').value,
		port: $('port').value || DEFAULTS.port,
		minSizeMb: $('minSizeMb').value,
		extensions: $('extensions').value.split(','),
		fallback: $('fallback').checked
	});
	await chrome.storage.local.set(o);
	await load();
	status('Saved.');
});

$('test').addEventListener('click', async () => {
	status('Testing…');
	const reply = await chrome.runtime.sendMessage({ type: 'ping' });
	status(reply?.message ?? 'No answer.', Boolean(reply?.ok));
});

load();
