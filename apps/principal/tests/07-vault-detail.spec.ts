// Vault detail page tests — spec J3 (cooling-off), spec J5 (drill mode), spec J6 (cancel release)
//
// The vault detail page surfaces the three principal actions on a vault:
// Run Drill, Force Release, and Cancel Release (COOLING_OFF only). It also
// lists sealed Letters in the vault. Actions are verified with API mocks.

import { test, expect } from '@playwright/test';
import { BASE, injectSession, MOCK_VAULT, MOCK_SUBSCRIPTION, MOCK_PLANS } from './fixtures';

const VAULT_URL = `${BASE}/vaults/vault-001`;

function mockVaultLoad(page: Parameters<typeof injectSession>[0], vaultOverride = {}) {
  return Promise.all([
    page.route('**/v1/vaults/vault-001', route =>
      route.fulfill({ json: { ...MOCK_VAULT, ...vaultOverride } })
    ),
    page.route('**/v1/vaults/vault-001/letters', route =>
      route.fulfill({ json: [] })
    ),
  ]);
}

test.describe('Vault detail — ACTIVE state (/vaults/vault-001)', () => {
  test.beforeEach(async ({ page }) => {
    await mockVaultLoad(page);
    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
  });

  test('shows vault name as page heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Family Vault' })).toBeVisible();
  });

  test('shows Active state badge', async ({ page }) => {
    // StatusBadge maps 'ACTIVE' → 'Active'
    await expect(page.getByText('Active')).toBeVisible();
  });

  test('shows "Run drill" and "Force release" buttons when ACTIVE', async ({ page }) => {
    await expect(page.getByRole('button', { name: /run drill/i })).toBeVisible();
    await expect(page.getByRole('button', { name: /force release/i })).toBeVisible();
  });

  test('"Cancel release" button absent when ACTIVE', async ({ page }) => {
    await expect(page.getByRole('button', { name: /cancel release/i })).not.toBeVisible();
  });

  test('Run drill POSTs to /drills and shows "Drill started" banner', async ({ page }) => {
    // doDrill has no confirm() guard — click fires directly
    await page.route('**/v1/vaults/vault-001/drills', route =>
      route.fulfill({ json: { release_event_id: 'evt-001', cooling_off_ends_at: '2026-06-13T00:00:00Z' } })
    );
    await page.getByRole('button', { name: /run drill/i }).click();
    await expect(page.getByText(/drill started/i)).toBeVisible();
  });

  test('Force release: accept confirm → shows "Cooling-off started" banner', async ({ page }) => {
    // doForceRelease uses window.confirm() — must register handler BEFORE click
    page.on('dialog', d => d.accept());
    await page.route('**/v1/vaults/vault-001/force-release', route =>
      route.fulfill({ json: { release_event_id: 'evt-002', cooling_off_ends_at: '2026-06-13T00:00:00Z' } })
    );
    await page.getByRole('button', { name: /force release/i }).click();
    await expect(page.getByText(/cooling-off started/i)).toBeVisible();
  });

  test('Force release: dismiss confirm → API not called', async ({ page }) => {
    page.on('dialog', d => d.dismiss());
    let called = false;
    await page.route('**/v1/vaults/vault-001/force-release', route => {
      called = true;
      route.fulfill({ json: {} });
    });
    await page.getByRole('button', { name: /force release/i }).click();
    expect(called).toBe(false);
  });
});

test.describe('Vault detail — COOLING_OFF state', () => {
  test.beforeEach(async ({ page }) => {
    await mockVaultLoad(page, { state: 'COOLING_OFF', cooling_off_started_at: '2026-05-29T00:00:00Z' });
    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
  });

  test('shows "Cancel release" button when COOLING_OFF', async ({ page }) => {
    await expect(page.getByRole('button', { name: /cancel release/i })).toBeVisible();
  });

  test('"Run drill" and "Force release" absent when COOLING_OFF', async ({ page }) => {
    await expect(page.getByRole('button', { name: /run drill/i })).not.toBeVisible();
    await expect(page.getByRole('button', { name: /force release/i })).not.toBeVisible();
  });

  test('Cancel release POSTs to /cancel and shows "Release cancelled" banner', async ({ page }) => {
    // doCancel has no confirm() guard — click fires directly
    await page.route('**/v1/vaults/vault-001/cancel', route =>
      route.fulfill({ json: { ...MOCK_VAULT, state: 'ACTIVE', cooling_off_started_at: null } })
    );
    await page.getByRole('button', { name: /cancel release/i }).click();
    await expect(page.getByText(/release cancelled/i)).toBeVisible();
  });
});

test.describe('Vault detail — Letters list', () => {
  test('shows "No Letters in this Vault yet." when list is empty', async ({ page }) => {
    await mockVaultLoad(page);
    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
    await expect(page.getByText('No Letters in this Vault yet.')).toBeVisible();
  });

  test('shows letter title and recipient when letters exist', async ({ page }) => {
    await page.route('**/v1/vaults/vault-001', route =>
      route.fulfill({ json: MOCK_VAULT })
    );
    await page.route('**/v1/vaults/vault-001/letters', route =>
      route.fulfill({ json: [{
        id: 'let-001',
        title: 'My Will',
        recipient_email: 'bob@example.com',
        sealed_at: '2026-05-01T00:00:00Z',
        scheduled_release_at: null,
      }] })
    );
    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
    await expect(page.getByText('My Will')).toBeVisible();
    await expect(page.getByText('bob@example.com')).toBeVisible();
  });
});

// spec 05 "Attachment storage region" + spec 12 §1 gating.
test.describe('Vault detail — storage region (Estate+/Legacy)', () => {
  async function mockPlanContext(page: Parameters<typeof injectSession>[0], planId: string) {
    await page.route('**/v1/principals/me/subscription', route =>
      route.fulfill({ json: { ...MOCK_SUBSCRIPTION, plan_id: planId } })
    );
    await page.route('**/v1/plans', route => route.fulfill({ json: MOCK_PLANS }));
  }

  test('Storage card hidden on Estate (no multi_region)', async ({ page }) => {
    await mockVaultLoad(page);
    await mockPlanContext(page, 'estate_monthly_v2');
    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
    await expect(page.getByText('Attachment Storage Region')).toHaveCount(0);
  });

  test('Estate+ can move region; POSTs to /region and shows confirmation', async ({ page }) => {
    await mockVaultLoad(page);
    await mockPlanContext(page, 'estate_plus_monthly_v1');

    page.on('dialog', d => d.accept());
    let postedRegion: string | undefined;
    await page.route('**/v1/vaults/vault-001/region', route => {
      postedRegion = (route.request().postDataJSON() as { storage_region?: string }).storage_region;
      route.fulfill({ json: { ...MOCK_VAULT, storage_region: 'us-east-1', storage_region_label: 'N. Virginia, United States' } });
    });

    await injectSession(page);
    await page.goto(VAULT_URL, { waitUntil: 'networkidle' });
    await expect(page.getByText('Attachment Storage Region')).toBeVisible();
    await page.locator('.region-row select').selectOption('us-east-1');
    await page.getByRole('button', { name: /move region/i }).click();
    await expect(page.getByText(/storage region moved to/i)).toBeVisible();
    expect(postedRegion).toBe('us-east-1');
  });
});
