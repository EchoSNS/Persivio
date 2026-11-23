// Multi-selection over the visible task list, with file-manager conventions:
// click selects one, Ctrl+click toggles, Shift+click selects a range from the anchor,
// arrows move the focus (Shift extends). Pure functions over immutable values.

export type Selection = {
	readonly ids: ReadonlySet<string>;
	/** Where a Shift range starts. */
	readonly anchor: string | null;
	/** The row with keyboard focus. */
	readonly focus: string | null;
};

export const EMPTY: Selection = { ids: new Set(), anchor: null, focus: null };

export function single(id: string): Selection {
	return { ids: new Set([id]), anchor: id, focus: id };
}

function range(order: readonly string[], from: string, to: string): Set<string> {
	const a = order.indexOf(from);
	const b = order.indexOf(to);
	if (a < 0 || b < 0) return new Set([to]);
	const [lo, hi] = a <= b ? [a, b] : [b, a];
	return new Set(order.slice(lo, hi + 1));
}

export function click(
	sel: Selection,
	id: string,
	order: readonly string[],
	mods: { ctrl: boolean; shift: boolean }
): Selection {
	if (mods.shift && sel.anchor !== null) {
		const ids = range(order, sel.anchor, id);
		if (mods.ctrl) for (const x of sel.ids) ids.add(x);
		return { ids, anchor: sel.anchor, focus: id };
	}
	if (mods.ctrl) {
		const ids = new Set(sel.ids);
		if (ids.has(id)) ids.delete(id);
		else ids.add(id);
		return { ids, anchor: id, focus: id };
	}
	return single(id);
}

export function selectAll(order: readonly string[]): Selection {
	return { ids: new Set(order), anchor: order[0] ?? null, focus: order[0] ?? null };
}

/** Moves the focus by `delta` rows; with `extend`, selects the range from the anchor. */
export function move(
	sel: Selection,
	order: readonly string[],
	delta: number,
	extend: boolean
): Selection {
	if (order.length === 0) return EMPTY;
	const current = sel.focus === null ? -1 : order.indexOf(sel.focus);
	const start = current < 0 ? (delta > 0 ? -1 : order.length) : current;
	const index = Math.min(order.length - 1, Math.max(0, start + delta));
	const id = order[index] as string;
	if (extend && sel.anchor !== null && order.includes(sel.anchor)) {
		return { ids: range(order, sel.anchor, id), anchor: sel.anchor, focus: id };
	}
	return single(id);
}

/** Drops ids that are no longer visible (filtered out or removed). */
export function prune(sel: Selection, order: readonly string[]): Selection {
	const visible = new Set(order);
	const kept = [...sel.ids].filter((id) => visible.has(id));
	if (kept.length === sel.ids.size && (sel.focus === null || visible.has(sel.focus))) return sel;
	return {
		ids: new Set(kept),
		anchor: sel.anchor !== null && visible.has(sel.anchor) ? sel.anchor : null,
		focus: sel.focus !== null && visible.has(sel.focus) ? sel.focus : null
	};
}
