import { getCurrentWindow } from '@tauri-apps/api/window';

/**
 * The app draws its own title bar where the window has none (Windows, ADR-027). Elsewhere
 * the system's title bar stays and nothing here is shown.
 */
class AppWindow {
	/** Whether to show our own minimize, maximize and close buttons. */
	ownControls = $state(false);
	maximized = $state(false);

	async init(): Promise<void> {
		try {
			const w = getCurrentWindow();
			if (await w.isDecorated()) return;
			this.ownControls = true;
			this.maximized = await w.isMaximized();
			await w.onResized(() => void this.refresh());
		} catch {
			// Not in the app (tests, a plain browser).
		}
	}

	private async refresh(): Promise<void> {
		try {
			this.maximized = await getCurrentWindow().isMaximized();
		} catch {
			// Only the icon would be wrong.
		}
	}

	minimize = () => void getCurrentWindow().minimize();
	toggleMaximize = () => void getCurrentWindow().toggleMaximize();
	/** A normal close request, so closing to the tray still works. */
	close = () => void getCurrentWindow().close();
}

export const appWindow = new AppWindow();
