// Buddies page tests — spec 06 (Buddy attestation)
//
// The buddies page lets principals invite trusted contacts and revoke them.
// Important: revokeBuddy calls DELETE /v1/buddies/{id} — not /v1/principals/me/buddies/{id}.
// Do NOT call mockDashboardApis here — it also registers a buddies route that conflicts.

import { test, expect } from '@playwright/test';
import { BASE, injectSession, MOCK_BUDDY } from './fixtures';

function mockBuddiesList(
  page: Parameters<typeof injectSession>[0],
  buddies: unknown[] = []
) {
  return page.route('**/v1/principals/me/buddies', route => {
    if (route.request().method() === 'GET') {
      route.fulfill({ json: buddies });
    } else {
      route.fallback();
    }
  });
}

test.describe('Buddies page — structure (/buddies)', () => {
  test.beforeEach(async ({ page }) => {
    await mockBuddiesList(page, []);
    await injectSession(page);
    await page.goto(BASE + '/buddies', { waitUntil: 'networkidle' });
  });

  test('shows "BUDDIES" eyebrow', async ({ page }) => {
    await expect(page.getByText('BUDDIES', { exact: true })).toBeVisible();
  });

  test('shows "Trusted contacts" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Trusted contacts' })).toBeVisible();
  });

  test('shows "No Buddies yet." when list is empty', async ({ page }) => {
    await expect(page.getByText('No Buddies yet.')).toBeVisible();
  });
});

test.describe('Buddies page — non-empty roster', () => {
  test.beforeEach(async ({ page }) => {
    await mockBuddiesList(page, [MOCK_BUDDY]);
    await injectSession(page);
    await page.goto(BASE + '/buddies', { waitUntil: 'networkidle' });
  });

  test('shows buddy email', async ({ page }) => {
    await expect(page.getByText('alice@example.com')).toBeVisible();
  });

  test('shows "Confirmed" status for confirmed buddy', async ({ page }) => {
    await expect(page.getByText('Confirmed')).toBeVisible();
  });

  test('"Revoke" button shown for non-revoked buddy', async ({ page }) => {
    await expect(page.getByRole('button', { name: /revoke/i })).toBeVisible();
  });
});

test.describe('Buddies page — invite form', () => {
  test.beforeEach(async ({ page }) => {
    await mockBuddiesList(page, []);
    await injectSession(page);
    await page.goto(BASE + '/buddies', { waitUntil: 'networkidle' });
  });

  test('"Invite Buddy" submit disabled when email is empty', async ({ page }) => {
    await expect(page.getByRole('button', { name: /invite buddy/i })).toBeDisabled();
  });

  test('"Invite Buddy" submit enables after typing an email', async ({ page }) => {
    await page.getByLabel('Email').fill('bob@example.com');
    await expect(page.getByRole('button', { name: /invite buddy/i })).toBeEnabled();
  });

  test('successful invite shows "Buddy invited: email" banner', async ({ page }) => {
    await page.route('**/v1/principals/me/buddies', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({ json: {
          buddy: {
            id: 'bud-002', display_name: null, email: 'bob@example.com', phone: null,
            prompt_cadence_days: 90, last_response: null, last_response_at: null,
            confirmed: false, revoked: false,
          },
          confirmation_token_DEV_ONLY: 'tok-dev-123',
        }});
      } else {
        route.fulfill({ json: [] });
      }
    });
    await page.getByLabel('Email').fill('bob@example.com');
    await page.getByRole('button', { name: /invite buddy/i }).click();
    await expect(page.getByText(/buddy invited.*bob@example\.com/i)).toBeVisible();
  });

  test('after successful invite, buddy list is reloaded', async ({ page }) => {
    await page.route('**/v1/principals/me/buddies', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({ json: {
          buddy: {
            id: 'bud-002', display_name: null, email: 'bob@example.com', phone: null,
            prompt_cadence_days: 90, last_response: null, last_response_at: null,
            confirmed: false, revoked: false,
          },
          confirmation_token_DEV_ONLY: 'tok-dev-123',
        }});
      } else {
        route.fulfill({ json: [] });
      }
    });
    // waitForResponse must be registered BEFORE the action that triggers the reload GET
    const reloadPromise = page.waitForResponse(
      resp => resp.url().includes('/v1/principals/me/buddies') && resp.request().method() === 'GET'
    );
    await page.getByLabel('Email').fill('bob@example.com');
    await page.getByRole('button', { name: /invite buddy/i }).click();
    await reloadPromise; // Resolves when the post-invite reload GET completes
  });
});

test.describe('Buddies page — revoke', () => {
  test.beforeEach(async ({ page }) => {
    await mockBuddiesList(page, [MOCK_BUDDY]);
    await injectSession(page);
    await page.goto(BASE + '/buddies', { waitUntil: 'networkidle' });
  });

  test('API error on invite shown as banner', async ({ page }) => {
    await page.route('**/v1/principals/me/buddies', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({
          status: 422,
          headers: { 'content-type': 'application/problem+json' },
          json: {
            type: 'https://paschal.com/errors/invalid',
            title: 'Invalid email',
            status: 422,
            detail: 'Email address is already a Buddy.',
          },
        });
      } else {
        route.fulfill({ json: [MOCK_BUDDY] });
      }
    });
    await page.getByLabel('Email').fill('alice@example.com');
    await page.getByRole('button', { name: /invite buddy/i }).click();
    await expect(page.getByText(/email address is already a buddy/i)).toBeVisible();
  });

  test('Revoke: accept confirm → DELETE called', async ({ page }) => {
    // revoke() uses window.confirm() — register handler BEFORE click
    // Revoke URL is /v1/buddies/{id} — NOT /v1/principals/me/buddies/{id}
    page.on('dialog', d => d.accept());
    let called = false;
    await page.route('**/v1/buddies/bud-001', route => {
      called = true;
      route.fulfill({ json: { ...MOCK_BUDDY, revoked: true } });
    });
    await page.getByRole('button', { name: /revoke/i }).click();
    expect(called).toBe(true);
  });

  test('Revoke: dismiss confirm → DELETE not called', async ({ page }) => {
    page.on('dialog', d => d.dismiss());
    let called = false;
    await page.route('**/v1/buddies/bud-001', route => {
      called = true;
      route.fulfill({ json: {} });
    });
    await page.getByRole('button', { name: /revoke/i }).click();
    expect(called).toBe(false);
  });
});
