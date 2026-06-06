// Shared mock data and helpers for Playwright tests.
// All shapes mirror the types defined in src/lib/types.ts and src/lib/api.ts.
// Mock values are chosen to be realistic but not require a running backend.

import type { Page } from '@playwright/test';

export const BASE = '/app';

// ---------------------------------------------------------------------------
// Mock API response payloads — values taken from spec 12 plan ladder
// ---------------------------------------------------------------------------

export const MOCK_PLANS = [
  {
    plan_id: 'draft_v1',
    tier: 'Draft',
    cadence: 'free',
    display_price: 'Free',
    price_usd_minor: 0,
    storage_bytes: 104857600,        // 100 MB (spec 12 §1)
    retention_days: 365,             // 1 year (spec 12 §5)
    scheduled_horizon_days: 365,
    max_vaults: 1,
    max_letters_per_vault: 1,
    max_trustees: 1,
    allowed_signals: ['HEARTBEAT'],
    sms_recipients_allowed: false,
    priority_support: false,
    concierge_dunning: false,
    published_audit: false,
    multi_region: false,
    notes: ['Heartbeat signal only'],
  },
  {
    plan_id: 'estate_monthly_v2',
    tier: 'Estate',
    cadence: 'monthly',
    display_price: '$3 / mo',
    price_usd_minor: 300,
    storage_bytes: 1073741824,       // 1 GB (spec 12 §1)
    retention_days: 1095,            // 3 years (spec 12 §5)
    scheduled_horizon_days: 1095,
    max_vaults: 5,
    max_letters_per_vault: 25,
    max_trustees: 5,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: false,
    concierge_dunning: false,
    published_audit: false,
    multi_region: false,
    notes: [],
  },
  {
    plan_id: 'estate_annual_v2',
    tier: 'Estate',
    cadence: 'annual',
    display_price: '$30 / yr',
    price_usd_minor: 3000,
    storage_bytes: 1073741824,
    retention_days: 1095,
    scheduled_horizon_days: 1095,
    max_vaults: 5,
    max_letters_per_vault: 25,
    max_trustees: 5,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: false,
    concierge_dunning: false,
    published_audit: false,
    multi_region: false,
    notes: [],
  },
  {
    plan_id: 'estate_plus_monthly_v1',
    tier: 'Estate+',
    cadence: 'monthly',
    display_price: '$8 / mo',
    price_usd_minor: 800,
    storage_bytes: 10737418240,      // 10 GB (spec 12 §1)
    retention_days: 3650,            // 10 years (spec 12 §5)
    scheduled_horizon_days: 3650,
    max_vaults: 10,
    max_letters_per_vault: 100,
    max_trustees: 10,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: false,
    concierge_dunning: false,
    published_audit: false,
    multi_region: true,
    notes: [],
  },
  {
    plan_id: 'estate_plus_annual_v1',
    tier: 'Estate+',
    cadence: 'annual',
    display_price: '$80 / yr',
    price_usd_minor: 8000,
    storage_bytes: 10737418240,
    retention_days: 3650,
    scheduled_horizon_days: 3650,
    max_vaults: 10,
    max_letters_per_vault: 100,
    max_trustees: 10,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: false,
    concierge_dunning: false,
    published_audit: false,
    multi_region: true,
    notes: [],
  },
  {
    plan_id: 'legacy_monthly_v1',
    tier: 'Legacy',
    cadence: 'monthly',
    display_price: '$20 / mo',
    price_usd_minor: 2000,
    storage_bytes: 107374182400,     // 100 GB (spec 12 §1)
    retention_days: 9125,            // 25 years (spec 12 §5)
    scheduled_horizon_days: 9125,
    max_vaults: 25,
    max_letters_per_vault: 250,
    max_trustees: 15,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: true,
    concierge_dunning: true,
    published_audit: true,
    multi_region: true,
    notes: [],
  },
  {
    plan_id: 'legacy_annual_v1',
    tier: 'Legacy',
    cadence: 'annual',
    display_price: '$200 / yr',
    price_usd_minor: 20000,
    storage_bytes: 107374182400,
    retention_days: 9125,
    scheduled_horizon_days: 9125,
    max_vaults: 25,
    max_letters_per_vault: 250,
    max_trustees: 15,
    allowed_signals: ['HEARTBEAT', 'CDR', 'GOOGLE', 'MICROSOFT', 'APPLE', 'CALENDAR', 'EMAIL'],
    sms_recipients_allowed: true,
    priority_support: true,
    concierge_dunning: true,
    published_audit: true,
    multi_region: true,
    notes: [],
  },
];

export const MOCK_SUBSCRIPTION = {
  state: 'TRIALING',
  plan_id: 'estate_monthly_v2',
  started_at: '2026-05-01T00:00:00Z',
  trial_end_at: '2026-05-31T00:00:00Z',
  current_period_end: '2026-06-01T00:00:00Z',
  canceled_at: null,
  retention_until: null,
};

export const MOCK_USAGE = {
  plan_id: 'estate_monthly_v2',
  tier: 'Estate',
  storage_used_bytes: 0,
  storage_quota_bytes: 1073741824,
  plan_base_storage_bytes: 1073741824,
  extra_storage_bytes: 0,
  storage_pct: 0,
  vaults_used: 0,
  vaults_quota: 5,
  letters_quota_per_vault: 25,
  co_stewards_quota: 2,
  retention_days: 1095,
  scheduled_horizon_days: 1095,
};

export const MOCK_VAULT = {
  id: 'vault-001',
  name: 'Family Vault',
  tier: 'HONEST_OPERATOR',
  state: 'ACTIVE',
  cooling_off_seconds: 1209600,    // 14 days (spec J3 default)
  last_attestation_at: '2026-05-29T10:00:00Z',
  cooling_off_started_at: null,
  released_at: null,
  storage_region: 'ap-southeast-2',
  storage_region_label: 'Sydney, Australia',
};

export const MOCK_HEARTBEAT = {
  received_at: '2026-05-30T05:00:00Z',
};

export const MOCK_BUDDY = {
  id: 'bud-001',
  display_name: null,
  email: 'alice@example.com',
  phone: null,
  prompt_cadence_days: 90,
  last_response: null,
  last_response_at: null,
  confirmed: true,
  revoked: false,
};

export const MOCK_CO_STEWARD = {
  id: 'cs-001',
  display_name: null,
  email: 'alice@example.com',
  confirmed: true,
  revoked: false,
  last_viewed_at: null,
  created_at: '2026-05-01T00:00:00Z',
};

export const MOCK_SIGNUP = {
  principal_id: 'pid-test-001',
  session_token: 'tok-test-abc',
  subscription_state: 'TRIALING',
  trial_end_at: '2026-06-30T00:00:00Z',
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Inject a valid principal session into localStorage so the auth guard passes.
 *  Uses addInitScript so the session is present before any page script executes —
 *  no intermediate navigation needed, avoiding stray API requests that skew networkidle. */
export async function injectSession(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('paschal.session_token', 'tok-test-abc');
    localStorage.setItem('paschal.email', 'test@example.com');
    localStorage.setItem('paschal.principal_id', 'pid-test-001');
  });
}

/** Clear all Paschal auth state from localStorage. */
export async function clearSession(page: Page) {
  await page.addInitScript(() => {
    localStorage.removeItem('paschal.session_token');
    localStorage.removeItem('paschal.email');
    localStorage.removeItem('paschal.principal_id');
    localStorage.removeItem('paschal_co_steward_session');
  });
}

/** Inject a co-steward session into localStorage. */
export async function injectCoStewardSession(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('paschal_co_steward_session', JSON.stringify({
      token: 'cs-tok-test',
      co_steward_id: 'cs-001',
    }));
  });
}

/** Register route mocks for the standard API endpoints used by the dashboard. */
export async function mockDashboardApis(page: Page) {
  await page.route('**/v1/vaults', route =>
    route.fulfill({ json: [] })
  );
  await page.route('**/v1/principals/me/subscription', route =>
    route.fulfill({ json: MOCK_SUBSCRIPTION })
  );
  await page.route('**/v1/principals/me/buddies', route =>
    route.fulfill({ json: [] })
  );
  await page.route('**/v1/principals/me/usage', route =>
    route.fulfill({ json: MOCK_USAGE })
  );
}

/** Register the plans list mock used by the sign-in page. */
export async function mockPlansApi(page: Page) {
  await page.route('**/v1/plans', route =>
    route.fulfill({ json: MOCK_PLANS })
  );
}
