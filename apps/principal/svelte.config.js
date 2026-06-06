import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // Single-page-app mode: all routes pre-rendered to one entrypoint
    // (`index.html`) which Svelte then routes client-side. Lets us embed
    // the entire app inside the Rust binary at compile time.
    adapter: adapter({
      fallback: 'index.html',
      pages: 'build',
      assets: 'build',
      strict: true
    }),
    paths: {
      // The Beacon serves this SPA mounted at `/app`. Asset URLs are
      // emitted with that prefix.
      base: '/app'
    }
  }
};

export default config;
