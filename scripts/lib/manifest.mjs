// Builds the updater manifest (latest.json) from a release's files. Pure; tested with
// `node --test scripts/lib/`.

/** Which update package serves which platform. Keys as in tauri-plugin-updater. */
const PACKAGES = [
	// The plain key is the fallback; keys with an installer name win for that installer.
	[/_x64-setup\.exe$/, ['windows-x86_64', 'windows-x86_64-nsis']],
	[/_x64_en-US\.msi$/, ['windows-x86_64-msi']],
	[/universal\.app\.tar\.gz$/, ['darwin-aarch64', 'darwin-x86_64', 'darwin-aarch64-app', 'darwin-x86_64-app']],
	[/_amd64\.AppImage$/, ['linux-x86_64', 'linux-x86_64-appimage']],
	[/_amd64\.deb$/, ['linux-x86_64-deb']]
];
/** Without these an installed copy could not update. */
const REQUIRED = ['windows-x86_64', 'darwin-aarch64', 'darwin-x86_64', 'linux-x86_64'];

const decode = (base64) => Buffer.from(base64.trim(), 'base64');

/** The 8-byte key id in a minisign public key (base64 of its text file). */
export function publicKeyId(pubkey) {
	const line = decode(pubkey).toString('utf8').split('\n')[1] ?? '';
	return decode(line).subarray(2, 10).toString('hex');
}

/** Key id and version named by a signature file (base64 of minisign's text). */
export function signatureInfo(signature) {
	const lines = decode(signature).toString('utf8').split('\n');
	return {
		keyId: decode(lines[1] ?? '').subarray(2, 10).toString('hex'),
		version: lines[2]?.match(/(?:^|\t|: )version:([^\t\r\n]+)/)?.[1] ?? null
	};
}

/**
 * `files`: names of all release files. `signatures`: name of a package → contents of its
 * `.sig`. Throws with every problem found, so nothing half-right is published.
 */
export function buildManifest({ version, notes, date, baseUrl, files, signatures, pubkey }) {
	const problems = [];
	const platforms = {};
	const expectedKey = publicKeyId(pubkey);
	for (const [pattern, keys] of PACKAGES) {
		const name = files.find((f) => pattern.test(f));
		if (!name) continue;
		if (!name.includes(version)) problems.push(`${name} is not version ${version}`);
		const signature = signatures[name];
		if (!signature) {
			problems.push(`${name} has no signature`);
			continue;
		}
		const info = signatureInfo(signature);
		if (info.keyId !== expectedKey) problems.push(`${name} is signed with another key than tauri.conf.json trusts`);
		if (info.version !== version) problems.push(`${name} is signed for version ${info.version}`);
		const entry = { signature: signature.trim(), url: `${baseUrl}/${encodeURIComponent(name)}` };
		for (const key of keys) platforms[key] = entry;
	}
	for (const key of REQUIRED) {
		if (!platforms[key]) problems.push(`no update package for ${key}`);
	}
	if (problems.length > 0) throw new Error([...new Set(problems)].join('\n'));
	return { version, notes, pub_date: date, platforms };
}
