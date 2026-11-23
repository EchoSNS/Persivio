// Light / dark / system theme. The choice is a per-machine view preference, so it lives
// in the webview's localStorage (guarded: storage can be unavailable).

export type ThemeChoice = 'system' | 'light' | 'dark';

const KEY = 'persivio.theme';

function load(): ThemeChoice {
	try {
		const v = localStorage.getItem(KEY);
		return v === 'light' || v === 'dark' ? v : 'system';
	} catch {
		return 'system';
	}
}

class Theme {
	choice = $state<ThemeChoice>(load());
	#media: MediaQueryList | null = null;

	/** Applies the theme and follows the OS while the choice is `system`. */
	start(): () => void {
		this.#media = window.matchMedia('(prefers-color-scheme: dark)');
		const onChange = () => this.apply();
		this.#media.addEventListener('change', onChange);
		this.apply();
		return () => this.#media?.removeEventListener('change', onChange);
	}

	set(choice: ThemeChoice): void {
		this.choice = choice;
		try {
			localStorage.setItem(KEY, choice);
		} catch {
			// Not persisted; still applied for this session.
		}
		this.apply();
	}

	apply(): void {
		const dark = this.choice === 'dark' || (this.choice === 'system' && !!this.#media?.matches);
		document.documentElement.classList.toggle('dark', dark);
	}
}

export const theme = new Theme();
