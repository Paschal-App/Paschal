// Auth guard tests — spec 13 Part A (authentication principles)
//
// "Passkey is primary; everything else is convenience." The layout's $effect
// checks localStorage for a session and redirects unauthenticated users to
// /signin from any protected route. Public routes must remain accessible.

import { test, expect } from '@playwright/test';
import { BASE, clearSession, injectSession, mockDashboardApis } from './fixtures';

test.describe('Auth guard — unauthenticated access', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
  });

  const PROTECTED = [
    { path: '/dashboard',   label: 'dashboard' },
    { path: '/vaults/new',  label: 'new vault form' },
    { path: '/letters/new', label: 'compose letter' },
    { path: '/buddies',     label: 'buddies' },
    { path: '/account',     label: 'account' },
  ];

  for (const { path, label } of PROTECTED) {
    test(`${label} (${path}) → redirects to /signin`, async ({ page }) => {
      await page.goto(BASE + path, { waitUntil: 'networkidle' });
      await page.waitForURL(`**${BASE}/signin`);
      expect(page.url()).toContain('/signin');
    });
  }

  const PUBLIC = [
    { path: '/',                      label: 'landing page' },
    { path: '/signin',                label: 'sign-in' },
    { path: '/co-steward/sign-in',    label: 'co-steward sign-in' },
    { path: '/co-steward/confirm',    label: 'co-steward confirm' },
  ];

  for (const { path, label } of PUBLIC) {
    test(`${label} (${path}) → stays accessible without auth`, async ({ page }) => {
      await page.goto(BASE + path, { waitUntil: 'networkidle' });
      expect(page.url()).not.toContain('/signin?' );
      // Should not have been bounced to principal signin from a public page.
      if (path !== '/signin') {
        expect(page.url()).toContain(path);
      }
    });
  }

  test('no infinite redirect loop on /signin', async ({ page }) => {
    // Navigate twice; page should remain stable at /signin.
    await page.goto(BASE + '/signin', { waitUntil: 'networkidle' });
    await page.goto(BASE + '/signin', { waitUntil: 'networkidle' });
    expect(page.url()).toContain('/signin');
    expect(page.url()).not.toContain('/signin/signin');
  });

  test('co-steward/dashboard (no co-steward session) → co-steward/sign-in', async ({ page }) => {
    // The co-steward dashboard manages its own localStorage key; the principal
    // auth guard excludes it from the principal-session check and the dashboard
    // page itself redirects to the co-steward sign-in if no session is found.
    await page.goto(BASE + '/co-steward/dashboard', { waitUntil: 'networkidle' });
    await page.waitForURL(`**${BASE}/co-steward/sign-in`);
    expect(page.url()).toContain('/co-steward/sign-in');
  });
});

test.describe('Auth guard — authenticated access', () => {
  test.beforeEach(async ({ page }) => {
    await mockDashboardApis(page);
    await injectSession(page);
  });

  test('authenticated user reaches dashboard', async ({ page }) => {
    await page.goto(BASE + '/dashboard', { waitUntil: 'networkidle' });
    expect(page.url()).toContain('/dashboard');
    expect(page.url()).not.toContain('/signin');
  });

  test('authenticated user reaches new vault form', async ({ page }) => {
    await page.goto(BASE + '/vaults/new', { waitUntil: 'networkidle' });
    expect(page.url()).toContain('/vaults/new');
  });
});
