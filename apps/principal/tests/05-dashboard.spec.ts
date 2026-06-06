// Dashboard tests — spec 12 §4 (subscription states), spec F3 (heartbeat),
//                  spec J1 (letter authoring), spec J6 (cancel release)
//
// The dashboard is the principal's hub. It shows subscription state, Vault
// list, Buddy list, and the Heartbeat button. All data is loaded from the API.

import { test, expect } from '@playwright/test';
import {
  BASE, injectSession, mockDashboardApis,
  MOCK_SUBSCRIPTION, MOCK_USAGE, MOCK_VAULT,
} from './fixtures';

test.describe('Dashboard — structure and navigation', () => {
  test.beforeEach(async ({ page }) => {
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
  });

  test('shows "YOUR ACCOUNT" eyebrow', async ({ page }) => {
    await expect(page.getByText('YOUR ACCOUNT')).toBeVisible();
  });

  test('shows "Dashboard" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Dashboard' })).toBeVisible();
  });

  test('Heartbeat button is visible (spec F3)', async ({ page }) => {
    // spec F3: "User-facing baseline; missing N consecutive is the strongest dormancy signal."
    // The button sends POST /v1/heartbeats.
    await expect(page.getByRole('button', { name: /heartbeat/i })).toBeVisible();
  });

  test('"New Vault" button is visible (spec J1)', async ({ page }) => {
    // spec J1 hire criterion: "A workflow that converts intent → sealed artifact in under 30 minutes."
    await expect(page.getByRole('link', { name: /new vault/i })).toBeVisible();
  });

  test('"New Vault" links to /vaults/new', async ({ page }) => {
    const btn = page.getByRole('link', { name: /new vault/i });
    const href = await btn.getAttribute('href');
    expect(href).toContain('/vaults/new');
  });

  test('shows SUBSCRIPTION card', async ({ page }) => {
    await expect(page.getByText('SUBSCRIPTION')).toBeVisible();
  });

  test('shows VAULTS card', async ({ page }) => {
    // Use exact:true to avoid matching "Your Vaults" heading (getByText is case-insensitive substring by default).
    await expect(page.getByText('VAULTS', { exact: true }).first()).toBeVisible();
  });

  test('shows BUDDIES card', async ({ page }) => {
    // Use exact:true to avoid matching NavBar "Buddies" link and "Your Buddies" heading.
    await expect(page.getByText('BUDDIES', { exact: true }).first()).toBeVisible();
  });
});

test.describe('Dashboard — subscription state display (spec 12 §4)', () => {
  test('TRIALING subscription shows trial state', async ({ page }) => {
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({ json: { ...MOCK_SUBSCRIPTION, state: 'TRIALING' } })
    );
    await page.route('**/v1/vaults', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/usage', r => r.fulfill({ json: MOCK_USAGE }));
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText(/trial/i).first()).toBeVisible();
  });

  test('CANCELED subscription shows cancellation message', async ({ page }) => {
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({
        json: {
          ...MOCK_SUBSCRIPTION,
          state: 'CANCELED',
          canceled_at: '2026-05-01T00:00:00Z',
          retention_until: '2029-05-01T00:00:00Z',
        }
      })
    );
    await page.route('**/v1/vaults', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/usage', r => r.fulfill({ json: MOCK_USAGE }));
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    // spec 12 §4: "CANCELED → EXPIRED : retention_until elapsed"
    await expect(page.getByText(/cancelled/i).first()).toBeVisible();
    await expect(page.getByText(/releasable until/i)).toBeVisible();
  });
});

test.describe('Dashboard — Heartbeat (spec F3)', () => {
  test.beforeEach(async ({ page }) => {
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
  });

  test('clicking Heartbeat POSTs to /v1/heartbeats', async ({ page }) => {
    let called = false;
    await page.route('**/v1/heartbeats', route => {
      called = true;
      route.fulfill({ json: { received_at: '2026-05-30T06:00:00Z' } });
    });
    // waitForResponse must be set up BEFORE the action that triggers the request.
    const responsePromise = page.waitForResponse('**/v1/heartbeats');
    await page.getByRole('button', { name: /heartbeat/i }).click();
    await responsePromise;
    expect(called).toBe(true);
  });

  test('heartbeat success shows a confirmation banner', async ({ page }) => {
    await page.route('**/v1/heartbeats', route =>
      route.fulfill({ json: { received_at: '2026-05-30T06:00:00Z' } })
    );
    await page.getByRole('button', { name: /heartbeat/i }).click();
    // Dashboard shows "Heartbeat recorded at …" in a Banner kind="ok"
    await expect(page.getByText(/heartbeat recorded/i)).toBeVisible();
  });

  test('Heartbeat button is disabled while sending', async ({ page }) => {
    // Intercept and hold the request so the button stays in its loading state.
    let resolveRoute!: () => void;
    const held = new Promise<void>(res => { resolveRoute = res; });
    await page.route('**/v1/heartbeats', async route => {
      await held;
      route.fulfill({ json: { received_at: '2026-05-30T06:00:00Z' } });
    });

    const btn = page.getByRole('button', { name: /heartbeat|sending/i });
    await btn.click();
    await expect(page.getByRole('button', { name: /sending/i })).toBeDisabled();
    resolveRoute();
  });
});

test.describe('Dashboard — Vaults section (spec J1, J3)', () => {
  test('empty vault list shows prompt to create first vault', async ({ page }) => {
    // spec J1: "Letter creation completes in one sitting."
    // The dashboard nudges with a create-vault CTA when none exist.
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText(/no vaults yet/i)).toBeVisible();
    await expect(page.getByRole('link', { name: /a family vault/i })).toBeVisible();
  });

  test('vault list shows vault name and state badge', async ({ page }) => {
    await page.route('**/v1/vaults', r => r.fulfill({ json: [MOCK_VAULT] }));
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({ json: MOCK_SUBSCRIPTION })
    );
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/usage', r => r.fulfill({ json: MOCK_USAGE }));
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('Family Vault')).toBeVisible();
    // StatusBadge renders the vault state (ACTIVE)
    await expect(page.getByText('ACTIVE')).toBeVisible();
  });

  test('vault row "Open →" link navigates to vault detail', async ({ page }) => {
    await page.route('**/v1/vaults', r => r.fulfill({ json: [MOCK_VAULT] }));
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({ json: MOCK_SUBSCRIPTION })
    );
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/usage', r => r.fulfill({ json: MOCK_USAGE }));
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    const link = page.getByRole('link', { name: /open →/i });
    const href = await link.getAttribute('href');
    expect(href).toContain(`/vaults/${MOCK_VAULT.id}`);
  });
});

test.describe('Dashboard — Buddies section (spec 06 §Buddy attestation)', () => {
  test('empty buddies list shows explanation and invite link', async ({ page }) => {
    // spec 06: "≥ 2 distinct Buddies required to count toward the score"
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText(/no buddies yet/i)).toBeVisible();
    await expect(page.getByText(/at least two are needed/i)).toBeVisible();
    await expect(page.getByRole('link', { name: /invite a buddy/i })).toBeVisible();
  });

  test('non-empty buddies list shows email and status columns', async ({ page }) => {
    const buddy = {
      id: 'bud-001',
      display_name: 'Alice',
      email: 'alice@example.com',
      phone: null,
      prompt_cadence_days: 90,
      last_response: null,
      last_response_at: null,
      confirmed: true,
      revoked: false,
    };
    await page.route('**/v1/vaults', r => r.fulfill({ json: [] }));
    await page.route('**/v1/principals/me/subscription', r =>
      r.fulfill({ json: MOCK_SUBSCRIPTION })
    );
    await page.route('**/v1/principals/me/buddies', r => r.fulfill({ json: [buddy] }));
    await page.route('**/v1/principals/me/usage', r => r.fulfill({ json: MOCK_USAGE }));
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('alice@example.com')).toBeVisible();
    await expect(page.getByText('Confirmed')).toBeVisible();
  });
});

test.describe('Dashboard — Usage meters (spec 12 §6)', () => {
  test('shows storage usage meter', async ({ page }) => {
    // spec 12 §6: "dashboard surfaces storage and vault usage via GET /v1/principals/me/usage"
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    await expect(page.getByText('USAGE')).toBeVisible();
    // Use exact:true to avoid matching "VAULTS" eyebrow, "Your Vaults" heading, "No Vaults yet." paragraph.
    await expect(page.getByText('Storage', { exact: true })).toBeVisible();
    await expect(page.getByText('Vaults', { exact: true })).toBeVisible();
  });
});
