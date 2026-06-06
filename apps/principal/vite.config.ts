import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    // Proxy /v1/* and /claim and other Beacon endpoints during dev so we
    // hit the running Beacon without CORS concerns. In production the
    // frontend is served from the Beacon itself, so this is dev-only.
    proxy: {
      '/v1': 'http://127.0.0.1:8080',
      '/health': 'http://127.0.0.1:8080',
      '/livez': 'http://127.0.0.1:8080',
      '/readyz': 'http://127.0.0.1:8080',
      '/metrics': 'http://127.0.0.1:8080',
      '/openapi.yaml': 'http://127.0.0.1:8080',
      '/claim': 'http://127.0.0.1:8080'
    }
  }
});
