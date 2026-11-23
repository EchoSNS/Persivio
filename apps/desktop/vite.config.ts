import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

// Fixed port so tauri.conf.json's devUrl can point at it.
const port = 1420;

export default defineConfig({
	plugins: [
		tailwindcss(),
		// SPA: Tauri serves the files itself; there is no server to render pages.
		sveltekit({ adapter: adapter({ fallback: 'index.html' }) }),
		// Browser builds of Svelte and auto-cleanup for component tests.
		svelteTesting()
	],
	clearScreen: false,
	server: {
		port,
		strictPort: true,
		host: '127.0.0.1',
		// The Rust side is rebuilt by `tauri dev`; don't reload the page for its files.
		watch: { ignored: ['**/src-tauri/**'] }
	},
	test: {
		include: ['src/**/*.test.ts'],
		// Pure logic runs in Node; component tests opt into jsdom per file.
		environment: 'node'
	}
});
