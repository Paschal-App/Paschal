// Sign-in / sign-up page tests — spec 12 §1 (plan ladder), spec 13 Part A
//
// The sign-in page doubles as sign-up in the MVP ("Repeated entries with the
// same email return your existing account"). It must show the full plan ladder
// — Draft, Estate, Estate+, Legacy — and the competitor comparison table that
// spec J2 requires ("Tier choice is binary, with plain-language consequences
// shown").

import { test, expect } from '@playwright/test';
import { BASE, clearSession, mockPlansApi, MOCK_SIGNUP } from './fixtures';

test.describe('Sign-in page — plan ladder (spec 12 §1)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
    await mockPlansApi(page);
    await page.goto(BASE + '/signin', { waitUntil: 'networkidle' });
  });

  test('page heading instructs user to choose a plan', async ({ page }) => {
    await expect(page.getByText('CHOOSE A PLAN')).toBeVisible();
  });

  test('Draft tier (free) is shown', async ({ page }) => {
    // spec 12: "Draft is genuinely free. No card, no expiry, no upgrade-required nag."
    await expect(page.getByRole('heading', { name: 'Draft' })).toBeVisible();
    await expect(page.getByText('Free', { exact: true })).toBeVisible();
  });

  test('Estate tier is shown and is featured', async ({ page }) => {
    // spec 12: "Estate is the recommended default for new sign-ups."
    const estateCard = page.locator('article.featured');
    await expect(estateCard).toBeVisible();
    await expect(estateCard.getByRole('heading', { name: 'Estate' })).toBeVisible();
  });

  test('Estate+ tier is shown', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Estate+' })).toBeVisible();
  });

  test('Legacy tier is shown', async ({ page }) => {
    // spec 12: "Legacy's 25-year retention is the brand-defining commitment."
    await expect(page.getByRole('heading', { name: 'Legacy' })).toBeVisible();
  });

  test('four tiers are displayed (Draft → Estate → Estate+ → Legacy)', async ({ page }) => {
    // spec 12 §1 plan ladder: Draft → Estate → Estate+ → Legacy
    const tiers = page.locator('article.tier h2');
    const names = await tiers.allTextContents();
    expect(names).toContain('Draft');
    expect(names).toContain('Estate');
    expect(names).toContain('Estate+');
    expect(names).toContain('Legacy');
  });

  test('each paid tier shows both monthly and annual cadence options', async ({ page }) => {
    // spec 12 §1: "Annual saves ~17% on list price … encouraged at checkout"
    const cadenceLabels = page.locator('label.cad');
    const count = await cadenceLabels.count();
    // Estate + Estate+ + Legacy each contribute 2 cadence options = at least 6
    expect(count).toBeGreaterThanOrEqual(6);
  });
});

test.describe('Sign-in page — competitor comparison table (spec J2)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
    await mockPlansApi(page);
    await page.goto(BASE + '/signin', { waitUntil: 'networkidle' });
  });

  test('comparison table is visible', async ({ page }) => {
    // J2 hire criterion: "Two clearly-labelled tiers … plain-language consequences shown."
    // The page satisfies this via the "HOW WE COMPARE" section.
    await expect(page.getByText('HOW WE COMPARE')).toBeVisible();
  });

  test('comparison table names Bitwarden Emergency Access as a competitor', async ({ page }) => {
    await expect(page.getByText(/Bitwarden/i).first()).toBeVisible();
  });

  test('comparison table names Cipherwill as a competitor', async ({ page }) => {
    await expect(page.getByText(/Cipherwill/i)).toBeVisible();
  });

  test('comparison table names GoodTrust as a competitor', async ({ page }) => {
    await expect(page.getByText(/GoodTrust/i)).toBeVisible();
  });

  test('comparison table names Vault12 as a competitor', async ({ page }) => {
    await expect(page.getByText(/Vault12/i)).toBeVisible();
  });

  test('multi-signal dead-man\'s switch listed as differentiator', async ({ page }) => {
    // Confirms the table row from the spec text
    await expect(page.getByText(/multi-signal dead-man/i)).toBeVisible();
  });

  test('drill mode listed as differentiator', async ({ page }) => {
    // J5 hire criterion: "Drill mode … releases dummy letters to real beneficiaries"
    await expect(page.getByText(/drill mode/i)).toBeVisible();
  });
});

test.describe('Sign-in page — email sign-in form (spec 13 Part A)', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
    await mockPlansApi(page);
    await page.goto(BASE + '/signin', { waitUntil: 'networkidle' });
  });

  test('"Continue with your email" section is visible', async ({ page }) => {
    await expect(page.getByText('Continue with your email')).toBeVisible();
  });

  test('email input field is present', async ({ page }) => {
    await expect(page.getByLabel('Email')).toBeVisible();
  });

  test('submit button is disabled when email is empty', async ({ page }) => {
    // Prevents accidental empty submission.
    const btn = page.getByRole('button', { name: /continue/i });
    await expect(btn).toBeDisabled();
  });

  test('submit button enables after email is typed and ToS accepted', async ({ page }) => {
    await page.getByLabel('Email').fill('alice@example.com');
    await page.getByRole('checkbox').check();
    const btn = page.getByRole('button', { name: /continue/i });
    await expect(btn).toBeEnabled();
  });

  test('successful sign-in redirects to dashboard', async ({ page }) => {
    await page.route('**/v1/auth/signup', route =>
      route.fulfill({ json: MOCK_SIGNUP })
    );
    // Dashboard APIs needed after redirect.
    await page.route('**/v1/vaults', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({ json: { state: 'TRIALING', plan_id: 'estate_monthly_v2',
        started_at: '2026-05-01T00:00:00Z', trial_end_at: '2026-05-31T00:00:00Z',
        current_period_end: '2026-06-01T00:00:00Z', canceled_at: null, retention_until: null } })
    );
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/usage', r =>
      r.fulfill({ json: { plan_id: 'estate_monthly_v2', tier: 'Estate',
        storage_used_bytes: 0, storage_quota_bytes: 1073741824,
        plan_base_storage_bytes: 1073741824, extra_storage_bytes: 0,
        storage_pct: 0, vaults_used: 0, vaults_quota: 5,
        letters_quota_per_vault: 25, co_stewards_quota: 2,
        retention_days: 1095, scheduled_horizon_days: 1095 } })
    );

    await page.getByLabel('Email').fill('alice@example.com');
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: /continue/i }).click();
    await page.waitForURL(`**${BASE}/dashboard`);
    expect(page.url()).toContain('/dashboard');
  });

  test('API error is shown as a banner', async ({ page }) => {
    await page.route('**/v1/auth/signup', route =>
      route.fulfill({
        status: 422,
        headers: { 'content-type': 'application/problem+json' },
        json: { type: 'https://paschal.com/errors/invalid',
          title: 'Invalid email', status: 422, detail: 'Not a valid email address.' },
      })
    );
    await page.getByLabel('Email').fill('bad@example.com');
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: /continue/i }).click();
    await expect(page.getByText(/not a valid email address/i)).toBeVisible();
  });
});
