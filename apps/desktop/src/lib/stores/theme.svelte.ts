// Light / dark / system theme and the glass effect. Both are per-machine view preferences,
// so they live in the webview's localStorage (guarded: storage can be unavailable).

export type ThemeChoice = 'system' | 'light' | 'dark';
export type SurfaceChoice = 'glass' | 'solid';

const THEME_KEY = 'persivio.theme';
const SURFACE_KEY = 'persivio.surface';

function load(key: string): string | null {
	try {
		return localStorage.getItem(key);
	} catch {
		return null;
	}
}

function store(key: string, value: string): void {
	try {
		localStorage.setItem(key, value);
	} catch {
		// Not persisted; still applied for this session.
	}
}

function loadTheme(): ThemeChoice {
	const v = load(THEME_KEY);
	return v === 'light' || v === 'dark' ? v : 'system';
}

class Theme {
	choice = $state<ThemeChoice>(loadTheme());
	surface = $state<SurfaceChoice>(load(SURFACE_KEY) === 'solid' ? 'solid' : 'glass');
	#dark: MediaQueryList | null = null;
	#opaque: MediaQueryList | null = null;

	/** Applies the theme and follows the OS (color scheme, reduced transparency). */
	start(): () => void {
		this.#dark = window.matchMedia('(prefers-color-scheme: dark)');
		this.#opaque = window.matchMedia(
			'(prefers-reduced-transparency: reduce), (prefers-contrast: more)'
		);
		const onChange = () => this.apply();
		this.#dark.addEventListener('change', onChange);
		this.#opaque.addEventListener('change', onChange);
		this.apply();
		return () => {
			this.#dark?.removeEventListener('change', onChange);
			this.#opaque?.removeEventListener('change', onChange);
		};
	}

	set(choice: ThemeChoice): void {
		this.choice = choice;
		store(THEME_KEY, choice);
		this.apply();
	}

	setSurface(surface: SurfaceChoice): void {
		this.surface = surface;
		store(SURFACE_KEY, surface);
		this.apply();
	}

	apply(): void {
		const dark = this.choice === 'dark' || (this.choice === 'system' && !!this.#dark?.matches);
		const solid = this.surface === 'solid' || !!this.#opaque?.matches;
		document.documentElement.classList.toggle('dark', dark);
		document.documentElement.classList.toggle('solid', solid);
	}
}

export const theme = new Theme();
