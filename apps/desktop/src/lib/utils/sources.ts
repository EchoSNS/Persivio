// Recognizes what the user pasted or dropped: http(s) links, magnet links, .torrent files.

export type LinkKind = 'http' | 'magnet' | 'invalid';

/** What kind of link `text` is. Only http(s) and magnet are accepted (same rule as Rust). */
export function classifyLink(text: string): LinkKind {
	const value = text.trim();
	if (/^magnet:\?/i.test(value)) return 'magnet';
	try {
		const url = new URL(value);
		return url.protocol === 'http:' || url.protocol === 'https:' ? 'http' : 'invalid';
	} catch {
		return 'invalid';
	}
}

/** Splits pasted text into links: one per line (or separated by spaces), trimmed, unique. */
export function splitLinks(text: string): string[] {
	const seen = new Set<string>();
	for (const token of text.split(/\s+/)) {
		const link = token.trim();
		if (link.length > 0) seen.add(link);
	}
	return [...seen];
}

/** The links in a drag-and-drop payload: `text/uri-list` first, then plain text. */
export function linksFromDrop(data: Pick<DataTransfer, 'getData'>): string[] {
	const uriList = data.getData('text/uri-list');
	if (uriList) {
		// RFC 2483: one URI per line, `#` starts a comment line.
		return splitLinks(
			uriList
				.split(/\r?\n/)
				.filter((line) => !line.startsWith('#'))
				.join('\n')
		).filter((l) => classifyLink(l) !== 'invalid');
	}
	return splitLinks(data.getData('text/plain')).filter((l) => classifyLink(l) !== 'invalid');
}

/** Whether a dropped or picked file is a `.torrent` file. */
export function isTorrentFile(file: { name: string }): boolean {
	return file.name.toLowerCase().endsWith('.torrent');
}

/** Largest .torrent accepted (same limit as the engine). */
export const MAX_TORRENT_BYTES = 16 * 1024 * 1024;

/** Base64 of a file's bytes, for the `torrent_file` add source. */
export async function fileToBase64(file: Blob): Promise<string> {
	if (file.size > MAX_TORRENT_BYTES) throw new Error('The .torrent file is too large.');
	return bytesToBase64(new Uint8Array(await file.arrayBuffer()));
}

export function bytesToBase64(bytes: Uint8Array): string {
	let binary = '';
	// Chunks keep String.fromCharCode's argument list small.
	for (let i = 0; i < bytes.length; i += 0x8000) {
		binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
	}
	return btoa(binary);
}

/** Parses "Name: value" lines into headers; returns the line numbers that are invalid. */
export function parseHeaders(text: string): {
	headers: { name: string; value: string }[];
	invalidLines: number[];
} {
	const headers: { name: string; value: string }[] = [];
	const invalidLines: number[] = [];
	text.split(/\r?\n/).forEach((line, i) => {
		if (line.trim() === '') return;
		const colon = line.indexOf(':');
		const name = colon > 0 ? line.slice(0, colon).trim() : '';
		if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name)) {
			invalidLines.push(i + 1);
			return;
		}
		headers.push({ name, value: line.slice(colon + 1).trim() });
	});
	return { headers, invalidLines };
}
