// Response shapes from the Beacon. Mirror crates/beacon-core/src/lib.rs.
// Kept hand-written rather than auto-generated; the openapi.yaml is the
// machine-readable contract.

export type Tier = 'HONEST_OPERATOR' | 'ZERO_KNOWLEDGE';

export type VaultState =
  | 'ACTIVE'
  | 'SUSPICIOUS'
  | 'ALERT'
  | 'COOLING_OFF'
  | 'RELEASING'
  | 'RELEASED'
  | 'ARCHIVED';

export type SubscriptionState =
  | 'TRIALING'
  | 'ACTIVE'
  | 'PAST_DUE'
  | 'CANCELED'
  | 'EXPIRED'
  | 'DELETED';

export type BuddyResponse = 'WELL' | 'WORRIED' | 'UNABLE_TO_REACH';

export interface SignupResp {
  principal_id: string;
  session_token: string;
  subscription_state: SubscriptionState;
  trial_end_at: string;
  magic_token_DEV_ONLY?: string;
}

export interface Vault {
  id: string;
  name: string;
  tier: Tier;
  state: VaultState;
  cooling_off_seconds: number;
  last_attestation_at: string;
  cooling_off_started_at: string | null;
  released_at: string | null;
  storage_region: string;
  storage_region_label: string;
}

export interface Letter {
  id: string;
  title: string;
  recipient_email: string;
  sealed_at: string;
  scheduled_release_at: string | null;
}

export interface Attachment {
  id: string;
  original_filename: string;
  original_mime: string;
  original_size: number;
  transformed_mime: string;
  transformed_size: number;
  sha256_hex: string;
  transformer_notes: { notes?: string[] };
}

export interface LetterWithAttachments extends Letter {
  attachments: Attachment[];
}

export interface Subscription {
  state: SubscriptionState;
  plan_id: string;
  started_at: string;
  trial_end_at: string | null;
  current_period_end: string | null;
  canceled_at: string | null;
  retention_until: string | null;
}

export interface Buddy {
  id: string;
  display_name: string | null;
  email: string;
  phone: string | null;
  prompt_cadence_days: number;
  last_response: BuddyResponse | null;
  last_response_at: string | null;
  confirmed: boolean;
  revoked: boolean;
}

export interface BuddyInviteResp {
  buddy: Buddy;
  confirmation_token_DEV_ONLY?: string;
}

export interface ProblemDetails {
  type: string;
  title: string;
  status: number;
  detail: string;
}

export class ApiError extends Error {
  constructor(public readonly problem: ProblemDetails) {
    super(`${problem.title}: ${problem.detail}`);
    this.name = 'ApiError';
  }
}
