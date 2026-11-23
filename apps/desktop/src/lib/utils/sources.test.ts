import { describe, expect, it } from 'vitest';
import {
	bytesToBase64,
	classifyLink,
	isTorrentFile,
	linksFromDrop,
	parseHeaders,
	splitLinks
} from './sources.ts';

describe('classifyLink', () => {
	it('accepts http(s) and magnet links only', () => {
		expect(classifyLink('https://example.com/a.iso')).toBe('http');
		expect(classifyLink('  http://example.com ')).toBe('http');
		expect(classifyLink('MAGNET:?xt=urn:btih:abc')).toBe('magnet');
		expect(classifyLink('ftp://example.com/a')).toBe('invalid');
		expect(classifyLink('file:///etc/passwd')).toBe('invalid');
		expect(classifyLink('javascript:alert(1)')).toBe('invalid');
		expect(classifyLink('not a link')).toBe('invalid');
	});
});

describe('splitLinks', () => {
	it('splits on lines and spaces, trims and removes duplicates', () => {
		expect(splitLinks('https://a/1\r\n\n  https://b/2 https://a/1\n')).toEqual([
			'https://a/1',
			'https://b/2'
		]);
		expect(splitLinks('   ')).toEqual([]);
	});
});

describe('linksFromDrop', () => {
	const drop = (types: Record<string, string>) => ({ getData: (t: string) => types[t] ?? '' });
	it('prefers uri-list, skips comments and unsupported schemes', () => {
		expect(
			linksFromDrop(
				drop({
					'text/uri-list': '# comment\r\nhttps://a/1\r\nfile:///x\r\nmagnet:?xt=urn:btih:abc',
					'text/plain': 'https://ignored'
				})
			)
		).toEqual(['https://a/1', 'magnet:?xt=urn:btih:abc']);
		expect(linksFromDrop(drop({ 'text/plain': 'see https://a/1 now' }))).toEqual(['https://a/1']);
	});
});

describe('files and headers', () => {
	it('recognizes torrent files by extension', () => {
		expect(isTorrentFile({ name: 'Ubuntu.TORRENT' })).toBe(true);
		expect(isTorrentFile({ name: 'a.torrent.exe' })).toBe(false);
	});

	it('encodes bytes as base64', () => {
		expect(bytesToBase64(new TextEncoder().encode('d4:infoe'))).toBe('ZDQ6aW5mb2U=');
		expect(bytesToBase64(new Uint8Array(100_000)).length).toBe(133_336);
	});

	it('parses header lines and reports bad ones', () => {
		expect(parseHeaders('Authorization: Bearer x:y\n\nbad line\nX-A:1')).toEqual({
			headers: [
				{ name: 'Authorization', value: 'Bearer x:y' },
				{ name: 'X-A', value: '1' }
			],
			invalidLines: [3]
		});
	});
});
