// Typed HTTP client for the Paschal Beacon.
//
// Every call goes through `request()` so we get consistent error handling,
// auth header insertion, and problem+json parsing.

import { load as loadSession } from './session';
import {
  ApiError,
  type Attachment,
  type Buddy,
  type BuddyInviteResp,
  type BuddyResponse,
  type Letter,
  type LetterWithAttachments,
  type ProblemDetails,
  type SignupResp,
  type Subscription,
  type Vault
} from './types';

const BASE = ''; // Same-origin: the frontend is served by the Beacon (or via Vite proxy in dev).

async function request<T>(
  path: string,
  init: RequestInit & { auth?: boolean } = {}
): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.auth !== false) {
    const s = loadSession();
    if (s) headers.set('authorization', `Bearer ${s.token}`);
  }
  if (init.body && !(init.body instanceof FormData) && !headers.has('content-type')) {
    headers.set('content-type', 'application/json');
  }

  const resp = await fetch(`${BASE}${path}`, { ...init, headers });
  if (resp.status === 204 || resp.headers.get('content-length') === '0') {
    return undefined as T;
  }

  const ct = resp.headers.get('content-type') || '';
  if (!resp.ok) {
    if (ct.includes('problem+json') || ct.includes('json')) {
      const problem = (await resp.json()) as ProblemDetails;
      throw new ApiError(problem);
    }
    throw new ApiError({
      type: 'https://paschal.com/errors/unknown',
      title: 'unknown',
      status: resp.status,
      detail: await resp.text().catch(() => '')
    });
  }
  if (ct.includes('json')) {
    return (await resp.json()) as T;
  }
  return (await resp.text()) as unknown as T;
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

export function signup(args: { email: string; display_name?: string; plan?: string; tos_accepted?: boolean }) {
  return request<SignupResp>('/v1/auth/signup', {
    method: 'POST',
    auth: false,
    body: JSON.stringify(args)
  });
}

export interface PublicPlan {
  plan_id: string;
  tier: string;
  cadence: 'free' | 'monthly' | 'annual';
  display_price: string;
  price_usd_minor: number;
  storage_bytes: number;
  retention_days: number;
  scheduled_horizon_days: number;
  max_vaults: number;
  max_letters_per_vault: number;
  max_trustees: number;
  allowed_signals: string[];
  sms_recipients_allowed: boolean;
  priority_support: boolean;
  concierge_dunning: boolean;
  published_audit: boolean;
  multi_region: boolean;
  notes: string[];
}

export function listPlans() {
  return request<PublicPlan[]>('/v1/plans', { auth: false });
}

/// The storage regions a Vault's attachment blobs may be placed in. Mirrors
/// beacon_core::StorageRegion — keep in lockstep if regions are added.
export const STORAGE_REGIONS: { code: string; label: string }[] = [
  { code: 'ap-southeast-2', label: 'Sydney, Australia' },
  { code: 'eu-central-1', label: 'Frankfurt, Germany' },
  { code: 'us-east-1', label: 'N. Virginia, United States' }
];

// ---------------------------------------------------------------------------
// Vaults
// ---------------------------------------------------------------------------

export function listVaults() {
  return request<Vault[]>('/v1/vaults');
}

export function getVault(id: string) {
  return request<Vault>(`/v1/vaults/${encodeURIComponent(id)}`);
}

export function createVault(args: {
  name: string;
  cooling_off_seconds?: number;
  storage_region?: string;
}) {
  return request<Vault>('/v1/vaults', {
    method: 'POST',
    body: JSON.stringify({ ...args, tier: 'HONEST_OPERATOR' })
  });
}

export function moveVaultRegion(vaultId: string, storageRegion: string) {
  return request<Vault>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/region`,
    { method: 'POST', body: JSON.stringify({ storage_region: storageRegion }) }
  );
}

export function forceRelease(vaultId: string) {
  return request<{ release_event_id: string; cooling_off_ends_at: string }>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/force-release`,
    { method: 'POST' }
  );
}

export function cancelRelease(vaultId: string) {
  return request<Vault>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/cancel`,
    { method: 'POST' }
  );
}

export function runDrill(vaultId: string) {
  return request<{ release_event_id: string; cooling_off_ends_at: string }>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/drills`,
    { method: 'POST' }
  );
}

// ---------------------------------------------------------------------------
// Letters
// ---------------------------------------------------------------------------

export function listLetters(vaultId: string) {
  return request<Letter[]>(`/v1/vaults/${encodeURIComponent(vaultId)}/letters`);
}

export function sealLetter(vaultId: string, args: {
  title: string;
  recipient_email: string;
  body: string;
  drill_body?: string;
  scheduled_release_at?: string;
  kind?: string;
  category?: string;
  release_mode?: string;
}) {
  return request<Letter>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/letters`,
    { method: 'POST', body: JSON.stringify(args) }
  );
}

export function uploadLetter(vaultId: string, args: {
  title: string;
  recipient_email: string;
  body?: string;
  files: File[];
  kind?: string;
  category?: string;
  release_mode?: string;
  scheduled_release_at?: string;
}) {
  const fd = new FormData();
  fd.append('title', args.title);
  fd.append('recipient_email', args.recipient_email);
  if (args.body) fd.append('body', args.body);
  if (args.kind) fd.append('kind', args.kind);
  if (args.category) fd.append('category', args.category);
  if (args.release_mode) fd.append('release_mode', args.release_mode);
  if (args.scheduled_release_at) fd.append('scheduled_release_at', args.scheduled_release_at);
  for (const f of args.files) fd.append('file', f, f.name);
  return request<LetterWithAttachments>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/letters/multipart`,
    { method: 'POST', body: fd }
  );
}

// ---------------------------------------------------------------------------
// Letter export (offline portable bundle)
// ---------------------------------------------------------------------------

export interface LetterExportBundle {
  format_version: number;
  letter_id: string;
  vault_id: string;
  title: string;
  recipient_email: string;
  sealed_at: string;
  scheduled_release_at: string | null;
  kind: string;
  category: string | null;
  release_mode: string;
  ciphertext_b64: string;
  nonce_b64: string;
  salt_b64: string;
  kdf: string;
  note: string;
}

export function exportLetter(vaultId: string, letterId: string, passphrase: string) {
  return request<LetterExportBundle>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/letters/${encodeURIComponent(letterId)}/export`,
    { method: 'POST', body: JSON.stringify({ passphrase }) }
  );
}

// ---------------------------------------------------------------------------
// Heartbeat
// ---------------------------------------------------------------------------

export function heartbeat() {
  return request<{ received_at: string }>('/v1/heartbeats', {
    method: 'POST',
    body: JSON.stringify({ via: 'WEB' })
  });
}

// ---------------------------------------------------------------------------
// Buddies
// ---------------------------------------------------------------------------

export function listBuddies() {
  return request<Buddy[]>('/v1/principals/me/buddies');
}

export function inviteBuddy(args: {
  email: string;
  display_name?: string;
  phone?: string;
  prompt_cadence_days?: number;
}) {
  return request<BuddyInviteResp>('/v1/principals/me/buddies', {
    method: 'POST',
    body: JSON.stringify(args)
  });
}

export function confirmBuddy(token: string) {
  return request<Buddy>('/v1/buddies/confirm', {
    method: 'POST',
    auth: false,
    body: JSON.stringify({ token })
  });
}

export function respondBuddy(buddyId: string, response: BuddyResponse) {
  return request<Buddy>(`/v1/buddies/${encodeURIComponent(buddyId)}/responses`, {
    method: 'POST',
    auth: false,
    body: JSON.stringify({ response })
  });
}

export function revokeBuddy(buddyId: string) {
  return request<{ revoked: boolean }>(
    `/v1/buddies/${encodeURIComponent(buddyId)}`,
    { method: 'DELETE' }
  );
}

// ---------------------------------------------------------------------------
// Co-Stewards (read-only family deputies)
// ---------------------------------------------------------------------------

export interface CoSteward {
  id: string;
  display_name: string | null;
  email: string;
  confirmed: boolean;
  revoked: boolean;
  last_viewed_at: string | null;
  created_at: string;
}

export interface InviteCoStewardResp {
  co_steward: CoSteward;
  confirmation_token_DEV_ONLY: string;
}

export function listCoStewards() {
  return request<CoSteward[]>('/v1/principals/me/co-stewards');
}

export function inviteCoSteward(args: { email: string; display_name?: string }) {
  return request<InviteCoStewardResp>('/v1/principals/me/co-stewards', {
    method: 'POST',
    body: JSON.stringify(args)
  });
}

export function revokeCoSteward(id: string) {
  return request<{ revoked: boolean }>(
    `/v1/co-stewards/${encodeURIComponent(id)}`,
    { method: 'DELETE' }
  );
}

export function confirmCoSteward(token: string, passphrase: string) {
  return request<{ co_steward_id: string; session_token: string }>(
    '/v1/co-stewards/confirm',
    {
      method: 'POST',
      auth: false,
      body: JSON.stringify({ token, passphrase })
    }
  );
}

export function signInCoSteward(email: string, passphrase: string) {
  return request<{ co_steward_id: string; session_token: string }>(
    '/v1/co-stewards/sign-in',
    {
      method: 'POST',
      auth: false,
      body: JSON.stringify({ email, passphrase })
    }
  );
}

export interface CoStewardDashboard {
  principal_email: string;
  subscription: Subscription;
  vaults: Vault[];
  buddy_count: number;
  letter_counts: { vault_id: string; letters: number }[];
}

export function coStewardDashboard() {
  return request<CoStewardDashboard>('/v1/co-stewards/me/dashboard');
}

// ---------------------------------------------------------------------------
// Subscription + account
// ---------------------------------------------------------------------------

export function getSubscription() {
  return request<Subscription>('/v1/principals/me/subscription');
}

export function cancelSubscription() {
  return request<Subscription>('/v1/principals/me/subscription/cancel', { method: 'POST' });
}

export interface Usage {
  plan_id: string;
  tier: string;
  storage_used_bytes: number;
  storage_quota_bytes: number;
  plan_base_storage_bytes: number;
  extra_storage_bytes: number;
  storage_pct: number;
  vaults_used: number;
  vaults_quota: number;
  letters_quota_per_vault: number;
  co_stewards_quota: number;
  retention_days: number;
  scheduled_horizon_days: number;
}

export function getUsage() {
  return request<Usage>('/v1/principals/me/usage');
}

export function buyStorageAddon(bundle: '1gb' | '5gb' | '10gb') {
  return request<{ checkout_url: string; stub: boolean }>(
    '/v1/principals/me/storage-addon',
    { method: 'POST', body: JSON.stringify({ bundle }) }
  );
}

export function openBillingPortal() {
  return request<{ checkout_url: string; stub: boolean }>(
    '/v1/principals/me/billing-portal',
    { method: 'POST' }
  );
}

export function reactivateSubscription() {
  return request<Subscription>(
    '/v1/principals/me/subscription/reactivate',
    { method: 'POST' }
  );
}

export function requestAccountDeletion() {
  return request<{ deletion_scheduled_for: string; cancel_until: string }>(
    '/v1/principals/me',
    { method: 'DELETE' }
  );
}

export function cancelAccountDeletion() {
  return request<{ cancelled: boolean }>(
    '/v1/principals/me/cancel-deletion',
    { method: 'POST' }
  );
}

// ---------------------------------------------------------------------------
// Vault contacts
// ---------------------------------------------------------------------------

export interface VaultContact {
  id: string;
  display_name: string;
  email: string;
  birthday: string | null;
  release_note: string | null;
  created_at: string;
}

export function listVaultContacts(vaultId: string) {
  return request<VaultContact[]>(`/v1/vaults/${encodeURIComponent(vaultId)}/contacts`);
}

export function createVaultContact(
  vaultId: string,
  body: Omit<VaultContact, 'id' | 'created_at'>
) {
  return request<VaultContact>(`/v1/vaults/${encodeURIComponent(vaultId)}/contacts`, {
    method: 'POST',
    body: JSON.stringify(body)
  });
}

export function updateVaultContact(
  vaultId: string,
  contactId: string,
  body: Omit<VaultContact, 'id' | 'created_at'>
) {
  return request<VaultContact>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/contacts/${encodeURIComponent(contactId)}`,
    { method: 'PUT', body: JSON.stringify(body) }
  );
}

export function deleteVaultContact(vaultId: string, contactId: string) {
  return request<void>(
    `/v1/vaults/${encodeURIComponent(vaultId)}/contacts/${encodeURIComponent(contactId)}`,
    { method: 'DELETE' }
  );
}

// ---------------------------------------------------------------------------
// Bank dormancy signal
// ---------------------------------------------------------------------------

export interface BankSignalStatus {
  webhook_url: string;
  last_webhook_at: string | null;
}

export function enrolBankSignal() {
  return request<BankSignalStatus>('/v1/principals/me/bank-signal', { method: 'POST' });
}

export function getBankSignalStatus() {
  return request<BankSignalStatus>('/v1/principals/me/bank-signal');
}

export function revokeBankSignal() {
  return request<void>('/v1/principals/me/bank-signal', { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// Re-exports so consumers don't have to import from two paths.
// ---------------------------------------------------------------------------

export { ApiError } from './types';
export type {
  Attachment,
  Buddy,
  BuddyResponse,
  Letter,
  LetterWithAttachments,
  Subscription,
  SubscriptionState,
  Vault,
  VaultState,
  Tier
} from './types';
