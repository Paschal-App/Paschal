// Co-Stewards management page tests — spec 05 (Co-Steward role)
//
// The co-stewards page lets principals invite read-only deputies and revoke them.
// Important: revokeCoSteward calls DELETE /v1/co-stewards/{id} — not /v1/principals/me/co-stewards/{id}.
// The page loads both co-stewards list AND subscription in parallel — both mocks required.

import { test, expect } from '@playwright/test';
import { BASE, injectSession, MOCK_SUBSCRIPTION, MOCK_CO_STEWARD } from './fixtures';

function mockCoStewardLoad(
  page: Parameters<typeof injectSession>[0],
  coStewards: unknown[] = []
) {
  return Promise.all([
    page.route('**/v1/principals/me/co-stewards', route => {
      if (route.request().method() === 'GET') {
        route.fulfill({ json: coStewards });
      } else {
        route.fallback();
      }
    }),
    page.route('**/v1/principals/me/subscription', route =>
      route.fulfill({ json: MOCK_SUBSCRIPTION })
    ),
  ]);
}

test.describe('Co-Stewards page — structure (/co-stewards)', () => {
  test.beforeEach(async ({ page }) => {
    await mockCoStewardLoad(page, []);
    await injectSession(page);
    await page.goto(BASE + '/co-stewards', { waitUntil: 'networkidle' });
  });

  test('shows "SHARING" eyebrow', async ({ page }) => {
    await expect(page.getByText('SHARING')).toBeVisible();
  });

  test('shows "Co-Stewards" heading', async ({ page }) => {
    // { exact: true } avoids matching "Current Co-Stewards" section heading
    await expect(page.getByRole('heading', { name: 'Co-Stewards', exact: true })).toBeVisible();
  });

  test('shows "No Co-Stewards yet." when list is empty', async ({ page }) => {
    await expect(page.getByText('No Co-Stewards yet.')).toBeVisible();
  });
});

test.describe('Co-Stewards page — non-empty roster', () => {
  test.beforeEach(async ({ page }) => {
    await mockCoStewardLoad(page, [MOCK_CO_STEWARD]);
    await injectSession(page);
    await page.goto(BASE + '/co-stewards', { waitUntil: 'networkidle' });
  });

  test('shows co-steward email', async ({ page }) => {
    await expect(page.getByText('alice@example.com')).toBeVisible();
  });

  test('shows "Confirmed" status badge for confirmed co-steward', async ({ page }) => {
    await expect(page.getByText('Confirmed')).toBeVisible();
  });

  test('"Revoke" button shown for non-revoked co-steward', async ({ page }) => {
    await expect(page.getByRole('button', { name: /revoke/i })).toBeVisible();
  });
});

test.describe('Co-Stewards page — invite form', () => {
  test.beforeEach(async ({ page }) => {
    await mockCoStewardLoad(page, []);
    await injectSession(page);
    await page.goto(BASE + '/co-stewards', { waitUntil: 'networkidle' });
  });

  test('"Send invitation" submit disabled when email is empty', async ({ page }) => {
    await expect(page.getByRole('button', { name: /send invitation/i })).toBeDisabled();
  });

  test('"Send invitation" submit enables after typing an email', async ({ page }) => {
    await page.getByLabel('Email').fill('bob@example.com');
    await expect(page.getByRole('button', { name: /send invitation/i })).toBeEnabled();
  });

  test('successful invite shows "Invited email" banner', async ({ page }) => {
    await page.route('**/v1/principals/me/co-stewards', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({ json: {
          co_steward: {
            id: 'cs-002', display_name: null, email: 'bob@example.com',
            confirmed: false, revoked: false,
            last_viewed_at: null, created_at: '2026-05-30T00:00:00Z',
          },
          confirmation_token_DEV_ONLY: 'tok-cs-dev-456',
        }});
      } else {
        route.fulfill({ json: [] });
      }
    });
    await page.getByLabel('Email').fill('bob@example.com');
    await page.getByRole('button', { name: /send invitation/i }).click();
    await expect(page.getByText(/invited.*bob@example\.com/i)).toBeVisible();
  });
});

test.describe('Co-Stewards page — revoke', () => {
  test.beforeEach(async ({ page }) => {
    await mockCoStewardLoad(page, [MOCK_CO_STEWARD]);
    await injectSession(page);
    await page.goto(BASE + '/co-stewards', { waitUntil: 'networkidle' });
  });

  test('API error on invite shown as banner', async ({ page }) => {
    await page.route('**/v1/principals/me/co-stewards', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({
          status: 422,
          headers: { 'content-type': 'application/problem+json' },
          json: {
            type: 'https://paschal.com/errors/invalid',
            title: 'Quota exceeded',
            status: 422,
            detail: 'Co-Steward quota exceeded for your plan.',
          },
        });
      } else {
        route.fulfill({ json: [MOCK_CO_STEWARD] });
      }
    });
    await page.getByLabel('Email').fill('bob@example.com');
    await page.getByRole('button', { name: /send invitation/i }).click();
    await expect(page.getByText(/co-steward quota exceeded/i)).toBeVisible();
  });

  test('Revoke: accept confirm → DELETE called', async ({ page }) => {
    // revoke() uses window.confirm() — register handler BEFORE click
    // Revoke URL is /v1/co-stewards/{id} — NOT /v1/principals/me/co-stewards/{id}
    page.on('dialog', d => d.accept());
    let called = false;
    await page.route('**/v1/co-stewards/cs-001', route => {
      called = true;
      route.fulfill({ json: { ...MOCK_CO_STEWARD, revoked: true } });
    });
    await page.getByRole('button', { name: /revoke/i }).click();
    expect(called).toBe(true);
  });

  test('Revoke: dismiss confirm → DELETE not called', async ({ page }) => {
    page.on('dialog', d => d.dismiss());
    let called = false;
    await page.route('**/v1/co-stewards/cs-001', route => {
      called = true;
      route.fulfill({ json: {} });
    });
    await page.getByRole('button', { name: /revoke/i }).click();
    expect(called).toBe(false);
  });
});
