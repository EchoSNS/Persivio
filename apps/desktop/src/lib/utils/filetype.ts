/** What people call a file, by its ending. */
const KINDS: [name: string, endings: string[]][] = [
	['Compressed archive', ['zip', '7z', 'rar', 'tar', 'gz', 'xz', 'bz2', 'zst', 'tgz']],
	['Disc image', ['iso', 'img']],
	['Program', ['exe', 'msi', 'msix', 'appx', 'com', 'scr', 'bat', 'cmd', 'ps1', 'vbs', 'js', 'jar', 'lnk', 'reg', 'hta', 'dmg', 'pkg', 'app', 'deb', 'rpm', 'appimage', 'sh', 'run', 'apk']],
	['Video', ['mp4', 'mkv', 'avi', 'mov', 'webm', 'm4v', 'wmv', 'mpg', 'mpeg']],
	['Music', ['mp3', 'flac', 'm4a', 'wav', 'ogg', 'opus', 'aac']],
	['Picture', ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'avif', 'bmp', 'tif', 'tiff']],
	['PDF document', ['pdf']],
	['Document', ['doc', 'docx', 'odt', 'rtf', 'txt', 'epub', 'md']],
	['Spreadsheet', ['xls', 'xlsx', 'ods', 'csv']],
	['Presentation', ['ppt', 'pptx', 'odp']],
	['Torrent file', ['torrent']]
];

/** Trailing dots and spaces are dropped by Windows: `setup.exe. ` is a program too. */
export function endingOf(fileName: string): string {
	const name = fileName.replace(/[. ]+$/, '');
	const dot = name.lastIndexOf('.');
	return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

function kindOf(fileName: string): string | undefined {
	const ending = endingOf(fileName);
	return KINDS.find(([, endings]) => endings.includes(ending))?.[0];
}

/** Whether opening the file would run something. */
export function isProgram(fileName: string): boolean {
	return kindOf(fileName) === 'Program';
}

/** `Compressed archive (.zip)`; without a known ending, what the browser says. */
export function describeType(fileName: string, mime: string | null): string {
	const ending = endingOf(fileName);
	const kind = kindOf(fileName);
	if (kind) return `${kind} (.${ending})`;
	if (ending) return `.${ending} file`;
	return mime && mime !== 'application/octet-stream' ? mime : 'Unknown type';
}
