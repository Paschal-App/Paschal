// Vault creation tests — spec J3, spec 12 §1
//
// "When I configure when the letters release, I want signals that genuinely
// correlate with my incapacity … so I can travel, log off, take a sabbatical."
// (J3). The cooling-off window is configurable, with a 14-day default (J3
// hire criterion: "Cooling-off window is configurable (14d default)").

import { test, expect } from '@playwright/test';
import { BASE, injectSession, MOCK_VAULT, MOCK_PLANS, MOCK_SUBSCRIPTION } from './fixtures';

test.describe('New Vault form (/vaults/new)', () => {
  test.beforeEach(async ({ page }) => {
    await injectSession(page);
    await page.goto(BASE + '/vaults/new', { waitUntil: 'networkidle' });
  });

  test('shows "NEW VAULT" eyebrow', async ({ page }) => {
    await expect(page.getByText('NEW VAULT')).toBeVisible();
  });

  test('shows "Create a Vault" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Create a Vault' })).toBeVisible();
  });

  test('has a Name field', async ({ page }) => {
    await expect(page.getByLabel('Name')).toBeVisible();
  });

  test('has a Cooling-off window field (spec J3 "configurable")', async ({ page }) => {
    await expect(page.getByLabel(/cooling-off window/i)).toBeVisible();
  });

  test('cooling-off default is 1,209,600 seconds (14 days) (spec J3)', async ({ page }) => {
    // spec J3 hire criterion: "Cooling-off window is configurable (14d default)"
    // 14 days = 14 × 86400 = 1,209,600 seconds
    const field = page.getByLabel(/cooling-off window/i);
    await expect(field).toHaveValue('1209600');
  });

  test('"Create Vault" submit button is present', async ({ page }) => {
    await expect(page.getByRole('button', { name: /create vault/i })).toBeVisible();
  });

  test('submit is disabled when Name is empty', async ({ page }) => {
    await expect(page.getByRole('button', { name: /create vault/i })).toBeDisabled();
  });

  test('submit enables after typing a name', async ({ page }) => {
    await page.getByLabel('Name').fill('Family Vault');
    await expect(page.getByRole('button', { name: /create vault/i })).toBeEnabled();
  });

  test('Cancel link returns to dashboard', async ({ page }) => {
    const cancel = page.getByRole('link', { name: 'Cancel' });
    const href = await cancel.getAttribute('href');
    expect(href).toContain('/dashboard');
  });

  test('successful vault creation redirects to vault detail page', async ({ page }) => {
    await page.route('**/v1/vaults', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({ json: MOCK_VAULT });
      } else {
        route.fulfill({ json: [] });
      }
    });
    // Vault detail page also calls list letters and get vault.
    await page.route(`**/v1/vaults/${MOCK_VAULT.id}`, route =>
      route.fulfill({ json: MOCK_VAULT })
    );
    await page.route(`**/v1/vaults/${MOCK_VAULT.id}/letters`, route =>
      route.fulfill({ json: [] })
    );

    await page.getByLabel('Name').fill('Family Vault');
    await page.getByRole('button', { name: /create vault/i }).click();
    await page.waitForURL(`**${BASE}/vaults/${MOCK_VAULT.id}`);
    expect(page.url()).toContain(`/vaults/${MOCK_VAULT.id}`);
  });

  test('API error is shown as a banner', async ({ page }) => {
    await page.route('**/v1/vaults', route => {
      if (route.request().method() === 'POST') {
        route.fulfill({
          status: 403,
          headers: { 'content-type': 'application/problem+json' },
          json: {
            type: 'https://paschal.com/errors/quota',
            title: 'Vault quota reached',
            status: 403,
            detail: 'Your Estate plan allows up to 5 Vaults.',
          },
        });
      } else {
        route.fulfill({ json: [] });
      }
    });

    await page.getByLabel('Name').fill('One Too Many');
    await page.getByRole('button', { name: /create vault/i }).click();
    await expect(page.getByText(/vault quota reached|up to 5 vaults/i)).toBeVisible();
  });

  test('form explains purpose of a Vault', async ({ page }) => {
    // The description inside the form card contextualises the concept.
    await expect(
      page.getByText(/a container for letters/i)
    ).toBeVisible();
  });
});

// spec 12 §1 + spec 05 "Attachment storage region": only Estate+/Legacy may
// choose where a Vault's attachment blobs live.
test.describe('Storage region picker (Estate+/Legacy gating)', () => {
  async function mockPlanContext(page, planId: string) {
    await page.route('**/v1/principals/me/subscription', route =>
      route.fulfill({ json: { ...MOCK_SUBSCRIPTION, plan_id: planId } })
    );
    await page.route('**/v1/plans', route => route.fulfill({ json: MOCK_PLANS }));
  }

  test('hidden on a plan without multi_region (Estate)', async ({ page }) => {
    await injectSession(page);
    await mockPlanContext(page, 'estate_monthly_v2');
    await page.goto(BASE + '/vaults/new', { waitUntil: 'networkidle' });
    await expect(page.getByText(/storage region for attachments/i)).toHaveCount(0);
  });

  test('shown on Estate+ and the chosen region is sent on create', async ({ page }) => {
    await injectSession(page);
    await mockPlanContext(page, 'estate_plus_monthly_v1');

    let postedRegion: string | undefined;
    await page.route('**/v1/vaults', route => {
      if (route.request().method() === 'POST') {
        postedRegion = (route.request().postDataJSON() as { storage_region?: string }).storage_region;
        route.fulfill({ json: { ...MOCK_VAULT, storage_region: 'eu-central-1', storage_region_label: 'Frankfurt, Germany' } });
      } else {
        route.fulfill({ json: [] });
      }
    });
    await page.route(`**/v1/vaults/${MOCK_VAULT.id}`, route => route.fulfill({ json: MOCK_VAULT }));
    await page.route(`**/v1/vaults/${MOCK_VAULT.id}/letters`, route => route.fulfill({ json: [] }));

    await page.goto(BASE + '/vaults/new', { waitUntil: 'networkidle' });
    const select = page.getByLabel(/storage region for attachments/i);
    await expect(select).toBeVisible();
    await select.selectOption('eu-central-1');
    await page.getByLabel('Name').fill('EU Vault');
    await page.getByRole('button', { name: /create vault/i }).click();
    await page.waitForURL(`**${BASE}/vaults/${MOCK_VAULT.id}`);
    expect(postedRegion).toBe('eu-central-1');
  });
});
