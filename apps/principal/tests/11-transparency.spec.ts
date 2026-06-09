// Transparency page tests — transparency log + warrant canary.
//
// /app/transparency is a public route (no auth required). It displays the
// warrant canary statement, a table of transparency log entries, and a
// self-hosted continuity note.

import { test, expect } from '@playwright/test';
import { BASE, clearSession } from './fixtures';

const URL = `${BASE}/transparency`;

const MOCK_CANARY = {
  statement:
    "As of this statement, the operator of this Paschal instance has not received any secret government order, search warrant, gag order, or national-security letter requiring them to compromise the security or integrity of any user's data, and has not been compelled to install any backdoor.",
  issued_at: '2026-06-01T00:00:00Z',
  next_update_by: '2026-09-01T00:00:00Z',
};

const MOCK_LOG = [
  {
    id: 'alpha-uuid-001-abcd',
    kind: 'PRINCIPAL_SIGNED_UP',
    ts: '2026-06-07T12:00:00Z',
    payload: { email_hash: 'abc123' },
  },
  {
    id: 'bravo-uuid-002-efgh',
    kind: 'VAULT_SEALED',
    ts: '2026-06-06T08:00:00Z',
    payload: {},
  },
];

async function mockApis(page: Parameters<typeof clearSession>[0]) {
  await page.route('**/v1/public/warrant-canary', route =>
    route.fulfill({ json: MOCK_CANARY })
  );
  await page.route('**/v1/public/transparency-log', route =>
    route.fulfill({ json: MOCK_LOG })
  );
}

// ---------------------------------------------------------------------------
// Accessibility — no auth required
// ---------------------------------------------------------------------------

test.describe('Transparency page — accessibility', () => {
  test('loads without a session (not redirected to sign-in)', async ({ page }) => {
    await clearSession(page);
    await mockApis(page);
    const res = await page.goto(URL);
    expect(res?.status()).toBeLessThan(400);
    await expect(page.getByRole('heading', { name: /Transparency/i })).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Layout and content
// ---------------------------------------------------------------------------

test.describe('Transparency page — layout', () => {
  test.beforeEach(async ({ page }) => {
    await mockApis(page);
    await page.goto(URL, { waitUntil: 'networkidle' });
  });

  test('shows "TRANSPARENCY" eyebrow', async ({ page }) => {
    await expect(page.getByText('TRANSPARENCY', { exact: true }).first()).toBeVisible();
  });

  test('shows "Transparency & Trust" heading', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Transparency & Trust' })).toBeVisible();
  });

  test('shows WARRANT CANARY card', async ({ page }) => {
    await expect(page.getByText('WARRANT CANARY', { exact: true }).first()).toBeVisible();
  });

  test('shows canary statement text', async ({ page }) => {
    await expect(page.getByText(/not received any secret government order/i)).toBeVisible();
  });

  test('shows issued_at date for canary', async ({ page }) => {
    await expect(page.getByText(/Issued/)).toBeVisible();
  });

  test('shows next_update_by date for canary', async ({ page }) => {
    await expect(page.getByText(/Next update by/)).toBeVisible();
  });

  test('shows "absence of the canary is the warning" notice', async ({ page }) => {
    await expect(page.getByText(/absence of the canary/i)).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Transparency log
// ---------------------------------------------------------------------------

test.describe('Transparency page — log entries', () => {
  test.beforeEach(async ({ page }) => {
    await mockApis(page);
    await page.goto(URL, { waitUntil: 'networkidle' });
  });

  test('shows TRANSPARENCY LOG card', async ({ page }) => {
    await expect(page.getByText('TRANSPARENCY LOG', { exact: true }).first()).toBeVisible();
  });

  test('log table shows event kinds from the mock data', async ({ page }) => {
    await expect(page.getByText('PRINCIPAL_SIGNED_UP')).toBeVisible();
    await expect(page.getByText('VAULT_SEALED')).toBeVisible();
  });

  test('log table shows truncated entry IDs', async ({ page }) => {
    // IDs are truncated to 8 chars + "…" — use distinct prefixes in mock data
    await expect(page.getByText('alpha-uu…')).toBeVisible();
    await expect(page.getByText('bravo-uu…')).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Continuity section (self-hosted)
// ---------------------------------------------------------------------------

test.describe('Transparency page — continuity (self-hosted)', () => {
  test.beforeEach(async ({ page }) => {
    await mockApis(page);
    await page.goto(URL, { waitUntil: 'networkidle' });
  });

  test('shows "CONTINUITY" card', async ({ page }) => {
    await expect(page.getByText('CONTINUITY', { exact: true }).first()).toBeVisible();
  });

  test('explains database backup', async ({ page }) => {
    await expect(page.getByText(/Database backup/i)).toBeVisible();
  });

  test('explains encryption-key backup', async ({ page }) => {
    await expect(page.getByText(/Encryption-key backup/i)).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Error states
// ---------------------------------------------------------------------------

test.describe('Transparency page — error states', () => {
  test('shows warning banner when canary API fails', async ({ page }) => {
    await page.route('**/v1/public/warrant-canary', route =>
      route.fulfill({ status: 503, json: { title: 'Service unavailable', status: 503, detail: 'Temporarily unavailable.' } })
    );
    await page.route('**/v1/public/transparency-log', route =>
      route.fulfill({ json: [] })
    );
    await page.goto(URL, { waitUntil: 'networkidle' });
    await expect(page.getByText(/unavailable|error/i).first()).toBeVisible();
  });

  test('shows empty state message when log is empty', async ({ page }) => {
    await page.route('**/v1/public/warrant-canary', route =>
      route.fulfill({ json: MOCK_CANARY })
    );
    await page.route('**/v1/public/transparency-log', route =>
      route.fulfill({ json: [] })
    );
    await page.goto(URL, { waitUntil: 'networkidle' });
    await expect(page.getByText(/No entries yet/i)).toBeVisible();
  });
});
