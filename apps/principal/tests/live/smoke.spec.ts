import { expect, test } from '@playwright/test';

// Smoke tests against the LIVE dev deployment (through the Cloudflare Access
// service token). These do not mock the API — they prove the deployed image is
// reachable and serving. Heavier persona/UAT coverage is the Claude-skill run.

test('backend readiness probe responds through the tunnel', async ({ request }) => {
  const res = await request.get('/readyz');
  expect(res.status()).toBe(200);
});

test('principal app shell loads behind Access', async ({ page }) => {
  const res = await page.goto('/app/signin');
  expect(res?.status(), 'sign-in page should load (not blocked by Access)').toBeLessThan(400);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
});
