// Account page tests — spec 12 §4 (subscription states), account deletion
//
// The account page surfaces subscription cancel/reactivate and account deletion.
// These are the highest-risk irreversible actions in the app. All actions that
// use window.confirm() require the dialog handler to be registered BEFORE click.
//
// Timing note: the subscription $effect fires after SvelteKit hydrates, which
// can be after Playwright's 'networkidle' is achieved. Use waitForResponse to
// ensure the subscription data is rendered before asserting.

import { test, expect } from '@playwright/test';
import { BASE, injectSession, MOCK_SUBSCRIPTION } from './fixtures';

async function gotoAccount(
  page: Parameters<typeof injectSession>[0],
  subOverride: Record<string, unknown> = {}
) {
  await page.route('**/v1/principals/me/subscription', route =>
    route.fulfill({ json: { ...MOCK_SUBSCRIPTION, ...subOverride } })
  );
  await injectSession(page);
  // waitForResponse must be set up BEFORE the navigation that triggers the request
  const subLoaded = page.waitForResponse('**/v1/principals/me/subscription');
  await page.goto(BASE + '/account', { waitUntil: 'load' });
  await subLoaded;
}

test.describe('Account page — structure (/account)', () => {
  test.beforeEach(async ({ page }) => {
    await gotoAccount(page);
  });

  test('shows "ACCOUNT" eyebrow', async ({ page }) => {
    // { exact: true } avoids matching NavBar "Account" link and "Delete your account" heading
    await expect(page.getByText('ACCOUNT', { exact: true })).toBeVisible();
  });

  test('shows "Subscription & data" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Subscription & data' })).toBeVisible();
  });

  test('shows plan_id from subscription', async ({ page }) => {
    await expect(page.getByText('estate_monthly_v2')).toBeVisible();
  });

  test('"Cancel Subscription" button visible when TRIALING', async ({ page }) => {
    await expect(page.getByRole('button', { name: /cancel subscription/i })).toBeVisible();
  });

  test('"Reactivate" button absent when TRIALING', async ({ page }) => {
    await expect(page.getByRole('button', { name: /^reactivate$/i })).not.toBeVisible();
  });

  test('"Request deletion" button visible', async ({ page }) => {
    await expect(page.getByRole('button', { name: /request deletion/i })).toBeVisible();
  });
});

test.describe('Account page — subscription actions', () => {
  test('Cancel: accept confirm → shows "Subscription cancelled" banner', async ({ page }) => {
    await gotoAccount(page);
    // cancel() uses window.confirm() — register handler BEFORE click
    page.on('dialog', d => d.accept());
    await page.route('**/v1/principals/me/subscription/cancel', route =>
      route.fulfill({ json: {
        ...MOCK_SUBSCRIPTION,
        state: 'CANCELED',
        canceled_at: '2026-05-30T00:00:00Z',
        retention_until: '2029-05-30T00:00:00Z',
      }})
    );
    await page.getByRole('button', { name: /cancel subscription/i }).click();
    await expect(page.getByText(/subscription cancelled/i)).toBeVisible();
  });

  test('Cancel: dismiss confirm → API not called', async ({ page }) => {
    await gotoAccount(page);
    page.on('dialog', d => d.dismiss());
    let called = false;
    await page.route('**/v1/principals/me/subscription/cancel', route => {
      called = true;
      route.fulfill({ json: {} });
    });
    await page.getByRole('button', { name: /cancel subscription/i }).click();
    expect(called).toBe(false);
  });

  test('"Reactivate" visible when CANCELED, "Cancel Subscription" absent', async ({ page }) => {
    await gotoAccount(page, {
      state: 'CANCELED',
      canceled_at: '2026-05-01T00:00:00Z',
      retention_until: '2029-05-01T00:00:00Z',
    });
    await expect(page.getByRole('button', { name: /^reactivate$/i })).toBeVisible();
    await expect(page.getByRole('button', { name: /cancel subscription/i })).not.toBeVisible();
  });

  test('Reactivate → shows "Subscription reactivated" banner', async ({ page }) => {
    // reactivate() has no confirm() guard
    await gotoAccount(page, {
      state: 'CANCELED',
      canceled_at: '2026-05-01T00:00:00Z',
      retention_until: '2029-05-01T00:00:00Z',
    });
    await page.route('**/v1/principals/me/subscription/reactivate', route =>
      route.fulfill({ json: { ...MOCK_SUBSCRIPTION, state: 'ACTIVE' } })
    );
    await page.getByRole('button', { name: /^reactivate$/i }).click();
    await expect(page.getByText(/subscription reactivated/i)).toBeVisible();
  });

  test('API error on cancel shown as banner', async ({ page }) => {
    await gotoAccount(page);
    page.on('dialog', d => d.accept());
    await page.route('**/v1/principals/me/subscription/cancel', route =>
      route.fulfill({
        status: 422,
        headers: { 'content-type': 'application/problem+json' },
        json: {
          type: 'https://paschal.com/errors/invalid',
          title: 'Cannot cancel',
          status: 422,
          detail: 'Subscription already cancelled.',
        },
      })
    );
    await page.getByRole('button', { name: /cancel subscription/i }).click();
    await expect(page.getByText(/subscription already cancelled/i)).toBeVisible();
  });
});

test.describe('Account page — danger zone', () => {
  test.beforeEach(async ({ page }) => {
    await gotoAccount(page);
  });

  test('Request deletion: accept confirm → shows "Deletion scheduled" banner', async ({ page }) => {
    // requestDelete() uses window.confirm() — register handler BEFORE click
    page.on('dialog', d => d.accept());
    // Use method guard so this doesn't intercept GET /v1/principals/me/subscription
    await page.route('**/v1/principals/me', route => {
      if (route.request().method() === 'DELETE') {
        route.fulfill({ json: {
          deletion_scheduled_for: '2026-06-29T00:00:00Z',
          cancel_until: '2026-06-29T00:00:00Z',
        }});
      } else {
        route.fallback();
      }
    });
    await page.getByRole('button', { name: /request deletion/i }).click();
    await expect(page.getByText(/deletion scheduled/i)).toBeVisible();
  });

  test('Request deletion: dismiss confirm → API not called', async ({ page }) => {
    page.on('dialog', d => d.dismiss());
    let called = false;
    await page.route('**/v1/principals/me', route => {
      if (route.request().method() === 'DELETE') {
        called = true;
        route.fulfill({ json: {} });
      } else {
        route.fallback();
      }
    });
    await page.getByRole('button', { name: /request deletion/i }).click();
    expect(called).toBe(false);
  });
});
