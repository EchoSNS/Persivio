// The "Which file do I need?" part of a release page. Pure; tested with
// `node --test scripts/lib/`.

const HEADING = '## Which file do I need?';

/** In the order people look for them: [file name, system, what it is]. */
const DOWNLOADS = [
	[
		/_x64-setup\.exe$/,
		'**Windows 10, 11**',
		'**Recommended.** Installs for your account only, without administrator rights.'
	],
	[
		/_x64_en-US\.msi$/,
		'Windows, managed PCs',
		'For IT departments that roll out software: installs for every account and needs administrator rights. The same app as the `.exe`; do not install both.'
	],
	[
		/_universal\.dmg$/,
		'**macOS 11 or newer**',
		'Apple silicon and Intel. Open it and drag Persivio to Applications.'
	],
	[
		/_amd64\.AppImage$/,
		'**Linux, any distribution**',
		'One file, no installation: make it executable (`chmod +x`) and run it.'
	],
	[/_amd64\.deb$/, 'Debian, Ubuntu, Mint', 'Smaller, installed with the package manager (`sudo apt install ./file.deb`).'],
	[
		/^Persivio-extension_.*\.zip$/,
		'Browser extension',
		'Brave, Chrome, Edge: unpack it and follow the README inside. Needs Persivio installed.'
	]
];

/** Files that are not for downloading by hand. */
const TECHNICAL = [
	[/^SHA256SUMS\.txt$/, 'Checksums: compare them with your file to see that it arrived intact.'],
	[/^latest\.json$/, 'Read by the app when it looks for updates.'],
	[/\.sig$/, 'Signatures: the app refuses an update without a valid one.'],
	[/\.app\.tar\.gz$/, 'The update package for macOS. For a new installation take the `.dmg`.']
];

const size = (bytes) => `${(bytes / 1024 / 1024).toFixed(bytes < 1024 * 1024 ? 2 : 0)} MiB`;

/**
 * Markdown for a release page. `assets`: `{ name, size }` of every file. Throws for a file
 * nothing describes, so a new kind of file cannot be published unexplained.
 */
export function describeDownloads({ assets, baseUrl }) {
	const unknown = assets.filter((a) => ![...DOWNLOADS, ...TECHNICAL].some(([pattern]) => pattern.test(a.name)));
	if (unknown.length > 0) throw new Error(`no description for ${unknown.map((a) => a.name).join(', ')}`);

	const rows = DOWNLOADS.flatMap(([pattern, system, text]) =>
		assets
			.filter((a) => pattern.test(a.name))
			.map((a) => `| ${system} | [${a.name}](${baseUrl}/${a.name}) (${size(a.size)}) | ${text} |`)
	);
	const technical = TECHNICAL.flatMap(([pattern, text]) => {
		const names = assets.filter((a) => pattern.test(a.name)).map((a) => a.name);
		if (names.length === 0) return [];
		const label = names.length === 1 ? `\`${names[0]}\`` : '`….sig`';
		return [`- ${label}: ${text}`];
	});
	return [
		HEADING,
		'',
		'| System | File | What it is |',
		'|---|---|---|',
		...rows,
		'',
		'Already have Persivio? You need none of these: Settings → Updates → **Check now**.',
		'',
		'The installers are not code-signed yet, so Windows and macOS warn when you open them',
		'(Windows: **More info → Run anyway**; macOS: System Settings → Privacy & Security →',
		'**Open Anyway**).',
		'',
		'<details><summary>The other files (nothing to download by hand)</summary>',
		'',
		...technical,
		'',
		'</details>',
		''
	].join('\n');
}

/** `body` with its download part added, or replaced if it has one. */
export function withDownloads(body, downloads) {
	const notes = body.replaceAll('\r\n', '\n').split(HEADING)[0].trim();
	return `${notes}\n\n${downloads}`;
}
