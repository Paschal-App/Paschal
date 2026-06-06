// Landing page tests — spec 01 (overview), spec 12 (Legacy brand promise)
//
// The landing page is the unauthenticated entry point. Spec 12 §1 defines the
// brand-defining commitment: "Twenty-five years of patience." The product
// description is "A digital letter of last resort" (spec 01).

import { test, expect } from '@playwright/test';
import { BASE, clearSession, injectSession, mockDashboardApis } from './fixtures';

test.describe('Landing page', () => {
  test.beforeEach(async ({ page }) => {
    await clearSession(page);
  });

  test('renders product name and tagline', async ({ page }) => {
    await page.goto(BASE + '/');
    await expect(page.locator('h1')).toHaveText('Paschal');
    await expect(page.getByText('A digital letter of last resort.', { exact: true })).toBeVisible();
  });

  test('shows the brand promise: 25-year patience line', async ({ page }) => {
    // spec 12 §1: "Legacy's 25-year retention is the brand-defining commitment."
    await page.goto(BASE + '/');
    await expect(
      page.getByText(/twenty-five years of patience/i)
    ).toBeVisible();
  });

  test('"Begin" CTA navigates to sign-in', async ({ page }) => {
    await page.goto(BASE + '/');
    await page.getByRole('link', { name: 'Begin' }).click();
    await page.waitForURL(`**${BASE}/signin`);
    expect(page.url()).toContain('/signin');
  });

  test('unauthenticated user is not redirected away from landing', async ({ page }) => {
    await page.goto(BASE + '/', { waitUntil: 'networkidle' });
    expect(page.url()).not.toContain('/signin');
    expect(page.url()).not.toContain('/dashboard');
  });

  test('authenticated user is redirected to dashboard', async ({ page }) => {
    // The landing page $effect calls goto('/dashboard') when a session exists.
    await mockDashboardApis(page);
    await injectSession(page);
    await page.goto(BASE + '/', { waitUntil: 'networkidle' });
    await page.waitForURL(`**${BASE}/dashboard`);
    expect(page.url()).toContain('/dashboard');
  });
});
