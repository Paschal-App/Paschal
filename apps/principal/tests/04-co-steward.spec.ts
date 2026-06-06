// Co-steward page tests — spec 05 (domain model: Co-Steward role)
//
// A Co-Steward is a read-only family deputy. They can see vault state, last
// heartbeat, signal strength, scheduled releases, and plan tier. They never
// see Letter contents and cannot change anything. (From co-steward/confirm
// page copy, which reflects the domain model.)
//
// Co-steward pages use their own localStorage key and auth flow entirely
// separate from the principal session.

import { test, expect } from '@playwright/test';
import { BASE, clearSession, injectCoStewardSession } from './fixtures';

test.describe('Co-steward confirm page (/co-steward/confirm)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
    await page.goto(BASE + '/co-steward/confirm', { waitUntil: 'networkidle' });
  });

  test('page is accessible without a principal session', async ({ page }) => {
    expect(page.url()).toContain('/co-steward/confirm');
  });

  test('shows "ACCEPT INVITATION" eyebrow', async ({ page }) => {
    await expect(page.getByText('ACCEPT INVITATION')).toBeVisible();
  });

  test('shows "Become a Co-Steward" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Become a Co-Steward' })).toBeVisible();
  });

  test('explains the co-steward role: read-only, no Letter contents', async ({ page }) => {
    // The page copy states: "never see Letter contents" and "cannot change anything"
    await expect(page.getByText(/never\s+see\s+Letter\s+contents/i).first()).toBeVisible();
    await expect(page.getByText(/cannot change anything/i)).toBeVisible();
  });

  test('has an invitation token field', async ({ page }) => {
    await expect(page.getByLabel('Invitation token')).toBeVisible();
  });

  test('token field is pre-filled from URL query param', async ({ page }) => {
    await page.goto(BASE + '/co-steward/confirm?token=my-invite-abc', {
      waitUntil: 'networkidle'
    });
    const tokenField = page.getByLabel('Invitation token');
    await expect(tokenField).toHaveValue('my-invite-abc');
  });

  test('has a passphrase field (≥ 10 chars enforced)', async ({ page }) => {
    await expect(page.getByLabel('Choose a passphrase')).toBeVisible();
  });

  test('has a confirm passphrase field', async ({ page }) => {
    await expect(page.getByLabel('Confirm passphrase')).toBeVisible();
  });

  test('submit is disabled without a token', async ({ page }) => {
    const btn = page.getByRole('button', { name: /accept and sign in/i });
    await expect(btn).toBeDisabled();
  });

  test('submit is disabled without a passphrase even with token', async ({ page }) => {
    await page.goto(BASE + '/co-steward/confirm?token=tok-abc', {
      waitUntil: 'networkidle'
    });
    const btn = page.getByRole('button', { name: /accept and sign in/i });
    await expect(btn).toBeDisabled();
  });

  test('passphrase mismatch shows an error', async ({ page }) => {
    await page.goto(BASE + '/co-steward/confirm?token=tok-abc', {
      waitUntil: 'networkidle'
    });
    await page.getByLabel('Choose a passphrase').fill('longpassphrase1');
    await page.getByLabel('Confirm passphrase').fill('doesnotmatch');
    await page.getByRole('button', { name: /accept and sign in/i }).click();
    await expect(page.getByText(/passphrases do not match/i)).toBeVisible();
  });

  test('short passphrase (< 10 chars) shows an error', async ({ page }) => {
    await page.goto(BASE + '/co-steward/confirm?token=tok-abc', {
      waitUntil: 'networkidle'
    });
    await page.getByLabel('Choose a passphrase').fill('short');
    await page.getByLabel('Confirm passphrase').fill('short');
    await page.getByRole('button', { name: /accept and sign in/i }).click();
    await expect(page.getByText(/must be at least 10 characters/i)).toBeVisible();
  });

  test('successful confirmation stores session and navigates to co-steward dashboard', async ({ page }) => {
    await page.route('**/v1/co-stewards/confirm', route =>
      route.fulfill({
        json: { co_steward_id: 'cs-001', session_token: 'cs-tok-abc' },
      })
    );
    await page.route('**/v1/co-stewards/me/dashboard', route =>
      route.fulfill({
        json: {
          principal_email: 'owner@example.com',
          subscription: {
            state: 'ACTIVE', plan_id: 'estate_monthly_v2',
            started_at: '2026-01-01T00:00:00Z', trial_end_at: null,
            current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null,
          },
          vaults: [],
          buddy_count: 2,
          letter_counts: [],
        },
      })
    );

    await page.goto(BASE + '/co-steward/confirm?token=tok-abc', {
      waitUntil: 'networkidle'
    });
    await page.getByLabel('Choose a passphrase').fill('mysecretphrase');
    await page.getByLabel('Confirm passphrase').fill('mysecretphrase');
    await page.getByRole('button', { name: /accept and sign in/i }).click();
    await page.waitForURL(`**${BASE}/co-steward/dashboard`);
    expect(page.url()).toContain('/co-steward/dashboard');
  });
});

test.describe('Co-steward sign-in page (/co-steward/sign-in)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
    await page.goto(BASE + '/co-steward/sign-in', { waitUntil: 'networkidle' });
  });

  test('page is accessible without a principal session', async ({ page }) => {
    expect(page.url()).toContain('/co-steward/sign-in');
  });

  test('shows "CO-STEWARD" eyebrow', async ({ page }) => {
    await expect(page.getByText('CO-STEWARD', { exact: true })).toBeVisible();
  });

  test('shows "Sign in to view a dashboard" heading', async ({ page }) => {
    await expect(
      page.getByRole('heading', { name: /sign in to view a dashboard/i })
    ).toBeVisible();
  });

  test('has email and passphrase fields', async ({ page }) => {
    await expect(page.getByLabel('Email')).toBeVisible();
    await expect(page.getByLabel('Passphrase')).toBeVisible();
  });

  test('submit is disabled without email or passphrase', async ({ page }) => {
    const btn = page.getByRole('button', { name: /sign in/i });
    await expect(btn).toBeDisabled();
  });

  test('has link back to principal sign-in', async ({ page }) => {
    // "Not a Co-Steward? Sign up as a Principal."
    await expect(page.getByRole('link', { name: /sign up as a principal/i })).toBeVisible();
  });

  test('principal sign-in link points to /signin', async ({ page }) => {
    const link = page.getByRole('link', { name: /sign up as a principal/i });
    const href = await link.getAttribute('href');
    expect(href).toContain('/signin');
  });

  test('successful sign-in stores session and navigates to co-steward dashboard', async ({ page }) => {
    await page.route('**/v1/co-stewards/sign-in', route =>
      route.fulfill({
        json: { co_steward_id: 'cs-001', session_token: 'cs-tok-abc' },
      })
    );
    await page.route('**/v1/co-stewards/me/dashboard', route =>
      route.fulfill({
        json: {
          principal_email: 'owner@example.com',
          subscription: {
            state: 'ACTIVE', plan_id: 'estate_monthly_v2',
            started_at: '2026-01-01T00:00:00Z', trial_end_at: null,
            current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null,
          },
          vaults: [],
          buddy_count: 0,
          letter_counts: [],
        },
      })
    );

    await page.getByLabel('Email').fill('deputy@example.com');
    await page.getByLabel('Passphrase').fill('mysecretpassphrase');
    await page.getByRole('button', { name: /sign in/i }).click();
    await page.waitForURL(`**${BASE}/co-steward/dashboard`);
    expect(page.url()).toContain('/co-steward/dashboard');
  });

  test('API error is shown as a banner', async ({ page }) => {
    await page.route('**/v1/co-stewards/sign-in', route =>
      route.fulfill({
        status: 401,
        headers: { 'content-type': 'application/problem+json' },
        json: { type: 'https://paschal.com/errors/unauthorized',
          title: 'Unauthorized', status: 401, detail: 'Invalid email or passphrase.' },
      })
    );
    await page.getByLabel('Email').fill('wrong@example.com');
    await page.getByLabel('Passphrase').fill('wrongpassphrase');
    await page.getByRole('button', { name: /sign in/i }).click();
    await expect(page.getByText(/invalid email or passphrase/i)).toBeVisible();
  });
});

test.describe('Co-steward dashboard (/co-steward/dashboard)', () => {
  test('without co-steward session → redirects to co-steward/sign-in', async ({ page }) => {
    await clearSession(page);
    await page.goto(BASE + '/co-steward/dashboard', { waitUntil: 'networkidle' });
    await page.waitForURL(`**${BASE}/co-steward/sign-in`);
    expect(page.url()).toContain('/co-steward/sign-in');
  });

  test('with co-steward session → shows read-only dashboard', async ({ page }) => {
    await clearSession(page);
    await page.route('**/v1/co-stewards/me/dashboard', route =>
      route.fulfill({
        json: {
          principal_email: 'owner@example.com',
          subscription: {
            state: 'ACTIVE', plan_id: 'estate_monthly_v2',
            started_at: '2026-01-01T00:00:00Z', trial_end_at: null,
            current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null,
          },
          vaults: [],
          buddy_count: 3,
          letter_counts: [],
        },
      })
    );
    await injectCoStewardSession(page);
    await page.goto(BASE + '/co-steward/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('CO-STEWARD VIEW')).toBeVisible();
    await expect(page.getByText('Read-only dashboard')).toBeVisible();
  });

  test('dashboard identifies the principal by email', async ({ page }) => {
    await clearSession(page);
    await page.route('**/v1/co-stewards/me/dashboard', route =>
      route.fulfill({
        json: {
          principal_email: 'owner@example.com',
          subscription: {
            state: 'ACTIVE', plan_id: 'estate_monthly_v2',
            started_at: '2026-01-01T00:00:00Z', trial_end_at: null,
            current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null,
          },
          vaults: [],
          buddy_count: 3,
          letter_counts: [],
        },
      })
    );
    await injectCoStewardSession(page);
    await page.goto(BASE + '/co-steward/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('owner@example.com')).toBeVisible();
  });

  test('sign-out clears session and navigates to co-steward/sign-in', async ({ page }) => {
    await clearSession(page);
    await page.route('**/v1/co-stewards/me/dashboard', route =>
      route.fulfill({
        json: {
          principal_email: 'owner@example.com',
          subscription: {
            state: 'ACTIVE', plan_id: 'estate_monthly_v2',
            started_at: '2026-01-01T00:00:00Z', trial_end_at: null,
            current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null,
          },
          vaults: [],
          buddy_count: 0,
          letter_counts: [],
        },
      })
    );
    await injectCoStewardSession(page);
    await page.goto(BASE + '/co-steward/dashboard', { waitUntil: 'networkidle' });
    await page.getByRole('button', { name: /sign out/i }).click();
    await page.waitForURL(`**${BASE}/co-steward/sign-in`);
    expect(page.url()).toContain('/co-steward/sign-in');

    // Session should be cleared from localStorage.
    const session = await page.evaluate(() =>
      localStorage.getItem('paschal_co_steward_session')
    );
    expect(session).toBeNull();
  });
});
