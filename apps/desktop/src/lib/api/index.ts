// The typed API. Everything comes from the generated bindings; this file only adds
// error handling helpers.

import type { CommandError } from './bindings.ts';

export { commands, events } from './bindings.ts';
export type * from './bindings.ts';

/** Whether a rejected command carried Persivio's typed error. */
export function isCommandError(e: unknown): e is CommandError {
	return (
		typeof e === 'object' &&
		e !== null &&
		'kind' in e &&
		'message' in e &&
		typeof (e as { message: unknown }).message === 'string'
	);
}

/** A message to show for any rejected command. */
export function errorMessage(e: unknown): string {
	if (isCommandError(e)) return e.message;
	if (e instanceof Error) return e.message;
	return String(e);
}
