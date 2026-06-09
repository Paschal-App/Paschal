// Getting Started onboarding card — shown on the dashboard until the principal
// has a Vault, a Buddy, and a proof-of-life signal (or dismisses it).

import { test, expect } from '@playwright/test';
import { BASE, injectSession, mockDashboardApis, MOCK_VAULT } from './fixtures';

test.describe('Getting Started — new account', () => {
  test.beforeEach(async ({ page }) => {
    await mockDashboardApis(page); // empty vaults + buddies
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
  });

  test('shows the GETTING STARTED card', async ({ page }) => {
    await expect(page.getByText('GETTING STARTED', { exact: true }).first()).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Set up your account' })).toBeVisible();
  });

  test('first incomplete step (create a Vault) shows its CTA', async ({ page }) => {
    await expect(page.getByRole('link', { name: 'Create a Vault' })).toBeVisible();
  });

  test('Skip dismisses the card', async ({ page }) => {
    await page.getByRole('button', { name: 'Skip' }).click();
    await expect(page.getByText('GETTING STARTED', { exact: true })).toHaveCount(0);
  });
});

test.describe('Getting Started — completed', () => {
  test('hidden once a Vault, Buddy and signal all exist', async ({ page }) => {
    await mockDashboardApis(page); // subscription + usage; overridden below (later routes win)
    await page.route('**/v1/vaults', r => r.fulfill({ json: [MOCK_VAULT] }));
    await page.route('**/v1/principals/me/buddies', r =>
      r.fulfill({ json: [{ id: 'b1', display_name: 'A', email: 'a@example.com', phone: null, prompt_cadence_days: 90, last_response: null, last_response_at: null, confirmed: true, revoked: false }] })
    );
    await page.route('**/v1/principals/me/bank-signal', r =>
      r.fulfill({ json: { webhook_url: 'http://localhost:8080/v1/signals/bank-dormancy/x', last_webhook_at: '2026-06-01T00:00:00Z' } })
    );
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('GETTING STARTED', { exact: true })).toHaveCount(0);
  });
});
