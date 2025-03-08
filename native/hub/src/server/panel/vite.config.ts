import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [sveltekit()],
	server: {
		proxy: {
			'/panel': { target: 'https://localhost:7863', secure: false, changeOrigin: true }
		}
	}
});
