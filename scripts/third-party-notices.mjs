// Collects the license texts of everything shipped inside Persivio. Their licenses (MIT,
// Apache-2.0, BSD, ...) require including the notices with the installers.
//
//   node scripts/third-party-notices.mjs [output file]

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = (path) => fileURLToPath(new URL(`../${path}`, import.meta.url));
const output = process.argv[2] ?? root('THIRD-PARTY-NOTICES.txt');

/** License files in a package folder. */
function licenseTexts(dir) {
	if (!existsSync(dir)) return [];
	return readdirSync(dir, { withFileTypes: true })
		.filter((f) => f.isFile() && /^(licen[cs]e|copying|notice|unlicense|copyright)/i.test(f.name))
		.map((f) => readFileSync(join(dir, f.name), 'utf8').replaceAll('\r\n', '\n').trim());
}

const packages = [];

// Rust: everything the app and the CLI link, without build- and test-only crates.
const metadata = JSON.parse(
	execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], { cwd: root('.'), maxBuffer: 1e9, encoding: 'utf8' })
);
const byId = new Map(metadata.packages.map((p) => [p.id, p]));
const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
const queue = metadata.packages.filter((p) => ['persivio-desktop', 'persivio-cli'].includes(p.name)).map((p) => p.id);
const seen = new Set(queue);
while (queue.length > 0) {
	for (const dep of nodes.get(queue.pop()).deps) {
		if (!dep.dep_kinds.some((k) => k.kind === null) || seen.has(dep.pkg)) continue;
		seen.add(dep.pkg);
		queue.push(dep.pkg);
	}
}
for (const id of seen) {
	const p = byId.get(id);
	if (p.source === null) continue; // our own crates
	packages.push({ name: `${p.name} ${p.version}`, license: p.license ?? 'see text', url: p.repository, texts: licenseTexts(dirname(p.manifest_path)) });
}

// JavaScript: what is bundled into the interface.
const app = root('apps/desktop');
const modules = new Set(Object.keys(JSON.parse(readFileSync(join(app, 'package.json'), 'utf8')).dependencies));
for (const name of ['svelte', '@sveltejs/kit', 'tailwindcss', '@tanstack/virtual-core']) modules.add(name);
for (const name of modules) {
	const dir = join(app, 'node_modules', name);
	const p = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
	packages.push({ name: `${p.name} ${p.version}`, license: p.license, url: p.repository?.url ?? p.homepage, texts: licenseTexts(dir) });
}

packages.sort((a, b) => a.name.localeCompare(b.name));
// Many packages share one text: print each text once, with everything it covers.
const texts = new Map();
const without = [];
for (const p of packages) {
	if (p.texts.length === 0) without.push(p);
	for (const t of p.texts) texts.set(t, [...(texts.get(t) ?? []), p.name]);
}
const line = '='.repeat(80);
let out = `Persivio includes the following software.\n\n`;
for (const p of packages) out += `${p.name}  (${p.license})${p.url ? `  ${p.url}` : ''}\n`;
if (without.length > 0) {
	out += `\nThese packages come without a license file; the license named above applies:\n`;
	out += without.map((p) => `  ${p.name} (${p.license})\n`).join('');
}
for (const [text, names] of texts) out += `\n${line}\n${names.join(', ')}\n${line}\n\n${text}\n`;
writeFileSync(output, out);
console.log(`${packages.length} packages, ${texts.size} texts, ${(out.length / 1024).toFixed(0)} KiB: ${output}`);
