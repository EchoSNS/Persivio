// Copies a finished draft release from the source repository to the public
// releases repository, as a draft there, with the updater manifest. Uses your own GitHub
// CLI login: no token is stored anywhere. See docs/release.md.
//
//   node scripts/publish-release.mjs v0.5.0             # creates the public draft
//   node scripts/publish-release.mjs v0.5.0 --publish   # makes it public: updates go out

import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildManifest } from './lib/manifest.mjs';

const SOURCE = 'EchoSNS/Persivio';
const PUBLIC = 'EchoSNS/persivio-releases';

const [tag, flag] = process.argv.slice(2);
const fail = (message) => {
	console.error(`error: ${message}`);
	process.exit(1);
};
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? '')) fail('usage: node scripts/publish-release.mjs vX.Y.Z [--publish]');

const installed = 'C:\\Program Files\\GitHub CLI\\gh.exe';
const GH = process.platform === 'win32' && existsSync(installed) ? installed : 'gh';
const gh = (...args) => execFileSync(GH, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 1e9 });

if (flag === '--publish') {
	const isDraft = gh('release', 'view', tag, '--repo', PUBLIC, '--json', 'isDraft', '-q', '.isDraft').trim();
	if (isDraft !== 'true') fail(`${tag} is not a draft in ${PUBLIC}`);
	gh('release', 'edit', tag, '--repo', PUBLIC, '--draft=false', '--latest');
	console.log(`Published https://github.com/${PUBLIC}/releases/tag/${tag}`);
	process.exit(0);
}

const root = (path) => fileURLToPath(new URL(`../${path}`, import.meta.url));
const version = tag.slice(1);
const dir = mkdtempSync(join(tmpdir(), 'persivio-release-'));
try {
	console.log(`Downloading ${tag} from ${SOURCE}…`);
	gh('release', 'download', tag, '--repo', SOURCE, '--dir', dir);
	const names = readdirSync(dir);

	// The checksums were made by the release workflow: the copy must be identical.
	const sums = readFileSync(join(dir, 'SHA256SUMS.txt'), 'utf8');
	for (const name of names.filter((n) => n !== 'SHA256SUMS.txt')) {
		const hash = createHash('sha256').update(readFileSync(join(dir, name))).digest('hex');
		if (!sums.includes(`${hash}  ${name}`)) fail(`${name} does not match SHA256SUMS.txt`);
	}

	const notes = gh('release', 'view', tag, '--repo', SOURCE, '--json', 'body', '-q', '.body').trim();
	const signatures = {};
	for (const name of names.filter((n) => n.endsWith('.sig'))) {
		signatures[name.slice(0, -4)] = readFileSync(join(dir, name), 'utf8');
	}
	const manifest = buildManifest({
		version,
		notes,
		date: new Date().toISOString(),
		baseUrl: `https://github.com/${PUBLIC}/releases/download/${tag}`,
		files: names,
		signatures,
		pubkey: JSON.parse(readFileSync(root('apps/desktop/src-tauri/tauri.conf.json'), 'utf8')).plugins.updater.pubkey
	});
	writeFileSync(join(dir, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
	writeFileSync(join(dir, 'notes.md'), `${notes}\n`);

	const files = names.filter((n) => n !== 'latest.json').map((n) => join(dir, n));
	gh('release', 'create', tag, '--repo', PUBLIC, '--draft', '--title', `Persivio ${version}`, '--notes-file', join(dir, 'notes.md'), ...files, join(dir, 'latest.json'));
	console.log(`\nPublic draft created: https://github.com/${PUBLIC}/releases`);
	console.log('Try an installer from it, then make it public:');
	console.log(`  node scripts/publish-release.mjs ${tag} --publish`);
} catch (e) {
	fail(e.message);
} finally {
	rmSync(dir, { recursive: true, force: true });
}
