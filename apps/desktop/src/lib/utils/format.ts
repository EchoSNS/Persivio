// Human-readable sizes, speeds and durations for the task list.

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** `1536` → `"1.5 KiB"`. Binary units, like the engine and most download managers. */
export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) return '—';
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const digits = unit === 0 || value >= 100 ? 0 : 1;
	return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/** Bytes per second → `"2.4 MiB/s"`. */
export function formatSpeed(bytesPerSecond: number): string {
	return `${formatBytes(bytesPerSecond)}/s`;
}

/** Seconds → `"1h 05m"`, `"4m 09s"`, `"12s"`; `null` → `"—"`. */
export function formatEta(seconds: number | null): string {
	if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return '—';
	const s = Math.round(seconds);
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const sec = s % 60;
	const pad = (n: number) => n.toString().padStart(2, '0');
	if (h > 0) return `${h}h ${pad(m)}m`;
	if (m > 0) return `${m}m ${pad(sec)}s`;
	return `${sec}s`;
}

/** Progress in percent (0–100), or `null` when the total is unknown. */
export function percent(downloaded: number, total: number | null): number | null {
	if (total === null) return null;
	if (total === 0) return 100;
	return Math.min(100, Math.max(0, (downloaded / total) * 100));
}
