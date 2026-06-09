// Duress signal tests — covert panic switch surfaced on the dashboard.
//
// The DURESS card lets the principal arm a private webhook URL. Triggering it
// freezes every Vault's release and alerts a contact. Three states: unarmed,
// armed (URL shown), and armed+triggered (frozen banner).

import { test, expect } from '@playwright/test';
import { BASE, injectSession, mockDashboardApis } from './fixtures';

const UNARMED = { armed: false, triggered_at: null, webhook_url: null, alert_email: null };
const ARMED = {
  armed: true,
  triggered_at: null,
  webhook_url: 'http://localhost:8080/v1/signals/duress/abc-123-def',
  alert_email: 'trusted@example.com',
};
const TRIGGERED = { ...ARMED, triggered_at: '2026-06-08T09:00:00Z' };

async function gotoDashboardWithDuress(page: Parameters<typeof injectSession>[0], duress: unknown) {
  await mockDashboardApis(page);
  await page.route('**/v1/principals/me/duress', route => {
    if (route.request().method() === 'GET') route.fulfill({ json: duress });
    else route.fulfill({ json: ARMED });
  });
  await injectSession(page);
  await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
}

test.describe('Duress card — unarmed', () => {
  test.beforeEach(({ page }) => gotoDashboardWithDuress(page, UNARMED));

  test('shows DURESS card', async ({ page }) => {
    await expect(page.getByText('DURESS', { exact: true }).first()).toBeVisible();
  });

  test('shows "no duress signal armed" empty state', async ({ page }) => {
    await expect(page.getByText(/no duress signal armed/i)).toBeVisible();
  });

  test('shows the "Arm duress signal" button', async ({ page }) => {
    await expect(page.getByRole('button', { name: /arm duress signal/i })).toBeVisible();
  });

  test('explains that triggering freezes every vault release', async ({ page }) => {
    await expect(page.getByText(/freezes every Vault's release/i)).toBeVisible();
  });
});

test.describe('Duress card — armed', () => {
  test.beforeEach(({ page }) => gotoDashboardWithDuress(page, ARMED));

  test('shows the private duress URL', async ({ page }) => {
    await expect(page.getByText(/your private duress URL/i)).toBeVisible();
    await expect(page.getByText(ARMED.webhook_url)).toBeVisible();
  });

  test('shows the "Disarm duress signal" control', async ({ page }) => {
    await expect(page.getByRole('button', { name: /disarm duress signal/i })).toBeVisible();
  });

  test('shows the gift-card how-to', async ({ page }) => {
    await expect(page.getByText(/connect it to a gift-card purchase/i)).toBeVisible();
  });
});

test.describe('Duress card — triggered', () => {
  test('shows the active/frozen banner', async ({ page }) => {
    await gotoDashboardWithDuress(page, TRIGGERED);
    await expect(page.getByText(/duress is active/i)).toBeVisible();
    await expect(page.getByText(/release is frozen/i)).toBeVisible();
  });
});

test.describe('Duress card — arming', () => {
  test('clicking Arm POSTs and reveals the private URL', async ({ page }) => {
    await gotoDashboardWithDuress(page, UNARMED);
    const responsePromise = page.waitForResponse(
      r => r.url().includes('/v1/principals/me/duress') && r.request().method() === 'POST'
    );
    await page.getByRole('button', { name: /arm duress signal/i }).click();
    await responsePromise;
    await expect(page.getByText(ARMED.webhook_url)).toBeVisible();
  });
});
