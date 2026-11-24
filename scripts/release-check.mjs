// Checks that a release is consistent before anything is built (release.yml), and writes
// release-notes.md from the changelog. Also runs locally: node scripts/release-check.mjs
//
// Environment: GITHUB_REF (a tag must match the version), HAS_KEY ("true" if the update
// signing key is configured), GITHUB_OUTPUT.

import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const read = (path) => readFileSync(fileURLToPath(new URL(`../${path}`, import.meta.url)), 'utf8');
const fail = (message) => {
	console.error(`error: ${message}`);
	process.exit(1);
};

const version = read('Cargo.toml').match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) fail('no version in Cargo.toml');

const ref = process.env.GITHUB_REF ?? '';
if (ref.startsWith('refs/tags/') && ref !== `refs/tags/v${version}`) {
	fail(`the tag is ${ref.slice(10)}, but Cargo.toml has version ${version}`);
}

for (const path of ['apps/desktop/package.json', 'extension/manifest.json']) {
	const other = JSON.parse(read(path)).version;
	if (other !== version) fail(`${path} has version ${other}, Cargo.toml has ${version}`);
}

const changelog = read('CHANGELOG.md').replaceAll('\r\n', '\n');
const start = changelog.indexOf(`## [${version}]`);
if (start < 0) fail(`CHANGELOG.md has no entry for ${version}`);
const end = changelog.indexOf('\n## [', start + 1);
const entry = changelog.slice(changelog.indexOf('\n', start) + 1, end < 0 ? undefined : end).trim();
if (!entry) fail(`the changelog entry for ${version} is empty`);
writeFileSync('release-notes.md', `${entry}\n`);

const signed = process.env.HAS_KEY === 'true';
if (signed) {
	const pubkey = (path) => JSON.parse(read(`apps/desktop/src-tauri/${path}`)).plugins.updater.pubkey;
	if (pubkey('tauri.conf.json') === pubkey('tauri.local.conf.json')) {
		fail(
			'a signing key is configured, but tauri.conf.json still trusts the local key (it has no ' +
				'password and must not sign public releases). See docs/release.md, step 1.'
		);
	}
} else {
	console.log('note: no update signing key: installers are built, but no update packages.');
}

console.log(`Persivio ${version}, ${signed ? 'signed updates' : 'no updates'}`);
if (process.env.GITHUB_OUTPUT) {
	appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\nsigned=${signed}\n`);
}
