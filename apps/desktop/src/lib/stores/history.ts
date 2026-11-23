// Speed history per task, for the detail drawer's sparkline: the last 60 progress
// samples (15 s at 4 Hz).

import type { ProgressView } from '#lib/api/bindings.ts';

export const HISTORY_LENGTH = 60;

/** Appends one sample per task in the batch. Mutates and returns `history`. */
export function recordSpeeds(
	history: Map<string, number[]>,
	batch: ProgressView[],
	max = HISTORY_LENGTH
): Map<string, number[]> {
	for (const p of batch) {
		const samples = history.get(p.id) ?? [];
		samples.push(p.speedBps);
		if (samples.length > max) samples.splice(0, samples.length - max);
		history.set(p.id, samples);
	}
	return history;
}

/** SVG path (`M x y L ...`) of `samples` scaled into a `width` × `height` box. */
export function sparklinePath(samples: readonly number[], width: number, height: number): string {
	if (samples.length === 0) return '';
	const peak = Math.max(...samples, 1);
	const step = samples.length > 1 ? width / (samples.length - 1) : 0;
	return samples
		.map((v, i) => {
			const x = Math.round(i * step * 10) / 10;
			const y = Math.round((height - (v / peak) * height) * 10) / 10;
			return `${i === 0 ? 'M' : 'L'}${x} ${y}`;
		})
		.join(' ');
}
