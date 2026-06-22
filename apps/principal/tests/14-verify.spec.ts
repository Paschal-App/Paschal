// Magic-link verify page tests — spec 13 Part A (passwordless email sign-in)
//
// The verify page reads the one-time token from the URL *fragment* (#token=) so
// it never reaches server logs or Referer headers, redeems it for a session,
// and redirects to the dashboard. A missing or rejected token shows a recovery
// banner that links back to the sign-in page.

import { test, expect } from '@playwright/test';
import { BASE, clearSession, mockDashboardApis } from './fixtures';

test.describe('Magic-link verify page (/auth/verify)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
  });

  test('fragment token is redeemed and redirects to the dashboard', async ({ page }) => {
    let verifyBody: unknown = null;
    await page.route('**/v1/auth/magic-link/verify', async route => {
      verifyBody = route.request().postDataJSON();
      await route.fulfill({
        json: {
          principal_id: 'pid-verify-001',
          session_token: 'tok-verify-001',
          email: 'returning@example.com'
        }
      });
    });
    await mockDashboardApis(page);

    await page.goto(BASE + '/auth/verify#token=magic-abc', { waitUntil: 'networkidle' });

    await page.waitForURL(`**${BASE}/dashboard`, { timeout: 15000 });
    expect(page.url()).toContain('/dashboard');
    // The token came from the fragment, not the query string.
    expect(verifyBody).toEqual({ token: 'magic-abc' });
  });

  test('missing token shows a recovery banner', async ({ page }) => {
    await page.goto(BASE + '/auth/verify', { waitUntil: 'networkidle' });
    await expect(page.getByText(/missing its token/i)).toBeVisible();
    await expect(page.getByRole('link', { name: /request a new sign-in link/i })).toBeVisible();
  });

  test('expired or already-used token shows the expired banner', async ({ page }) => {
    await page.route('**/v1/auth/magic-link/verify', route =>
      route.fulfill({
        status: 401,
        headers: { 'content-type': 'application/problem+json' },
        json: {
          type: 'https://paschal.com/errors/unauthorised',
          title: 'unauthorised',
          status: 401,
          detail: 'invalid token'
        }
      })
    );

    await page.goto(BASE + '/auth/verify#token=expired-token', { waitUntil: 'networkidle' });
    await expect(page.getByText(/expired or was already used/i)).toBeVisible();
  });
});
