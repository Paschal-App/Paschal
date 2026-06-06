import { defineConfig, devices } from '@playwright/test';

// The mocked `chromium` project runs against a local dev server with every API
// call stubbed. The `chromium-live` project runs a smaller smoke suite against
// the real dev deployment (through the Cloudflare Access service token) in CI —
// see tests/live/ and .github/workflows/cd.yml.
const liveBaseURL = process.env.PLAYWRIGHT_BASE_URL;
const cfId = process.env.CF_ACCESS_CLIENT_ID;
const cfSecret = process.env.CF_ACCESS_CLIENT_SECRET;

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  use: {
    baseURL: 'http://localhost:5173',
    headless: true,
    // SPA routing needs to settle before assertions.
    actionTimeout: 8000,
  },
  projects: [
    {
      name: 'chromium',
      testIgnore: /tests\/live\//,
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'chromium-live',
      testMatch: /tests\/live\/.*\.spec\.ts/,
      use: {
        ...devices['Desktop Chrome'],
        baseURL: liveBaseURL ?? 'http://localhost:5173',
        // Cloudflare Access service-token headers let CI through the tunnel.
        extraHTTPHeaders:
          cfId && cfSecret
            ? { 'CF-Access-Client-Id': cfId, 'CF-Access-Client-Secret': cfSecret }
            : {},
      },
    },
  ],
  // Dev server is already running (mocked) or remote (live); don't start it.
  webServer: undefined,
});
