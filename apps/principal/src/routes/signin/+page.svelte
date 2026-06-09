<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import {
    signup,
    listPlans,
    getPasskeyAuthOptions,
    verifyPasskeyAuthentication,
    ApiError,
    type PublicPlan
  } from '$lib/api';
  import { startPasskeyAuthentication } from '$lib/webauthn';
  import { save } from '$lib/session';
  import { fmtBytes } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let email = $state('');
  let plans = $state<PublicPlan[]>([]);
  let selected = $state<string>('estate_monthly_v2');
  let tosAccepted = $state(false);
  let submitting = $state(false);
  let passkeySubmitting = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    listPlans().then((p) => (plans = p)).catch((e) => {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    });
  });

  // Group plans for the ladder display.
  const tiers = $derived.by(() => {
    const order = ['Draft', 'Estate', 'Estate+', 'Legacy'];
    return order
      .map((tier) => {
        const inTier = plans.filter((p) => p.tier === tier);
        if (inTier.length === 0) return null;
        return { tier, plans: inTier };
      })
      .filter((x): x is { tier: string; plans: PublicPlan[] } => x !== null);
  });

  function yearsFromDays(d: number): string {
    if (d < 365) return `${d} days`;
    const y = Math.round(d / 365);
    return y === 1 ? '1 year' : `${y} years`;
  }

  async function submit(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    try {
      const r = await signup({ email, plan: selected, tos_accepted: tosAccepted });
      save({ token: r.session_token, email, principalId: r.principal_id });
      goto(`${base}/dashboard`);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      submitting = false;
    }
  }

  async function signInWithPasskey() {
    if (!email) { error = 'Enter your email first, then use your passkey.'; return; }
    passkeySubmitting = true;
    error = null;
    try {
      const { challenge_id, options } = await getPasskeyAuthOptions(email);
      const assertion = await startPasskeyAuthentication(options);
      const r = await verifyPasskeyAuthentication({ challenge_id, assertion });
      save({ token: r.session_token, email, principalId: r.principal_id });
      goto(`${base}/dashboard`);
    } catch (err) {
      if (err instanceof ApiError && err.problem.status === 404) {
        error = 'No passkey registered for this account. Sign in with email instead.';
      } else {
        error = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
      }
    } finally {
      passkeySubmitting = false;
    }
  }

  const selectedPlan = $derived(plans.find((p) => p.plan_id === selected));
</script>

<div class="page">
  <header class="head">
    <div class="eyebrow">CHOOSE A PLAN</div>
    <h1>What level of continuity do you need?</h1>
    <p class="lede">
      Every paid plan includes the full feature set. Tiers differ in storage,
      retention window, and how far into the future a Letter can be scheduled.
    </p>
    <p class="audit">
      Independent cryptographic review scheduled <strong>Q3 2026</strong>;
      report will be published in full, including any findings.
      <a href="/THREAT-MODEL.md" target="_blank" rel="noopener">Read the threat model</a>.
    </p>
  </header>

  {#if plans.length === 0}
    <p class="dim">Loading plans…</p>
  {:else}
    <div class="ladder">
      {#each tiers as t (t.tier)}
        {@const monthly = t.plans.find((p) => p.cadence === 'monthly')}
        {@const annual = t.plans.find((p) => p.cadence === 'annual')}
        {@const free = t.plans.find((p) => p.cadence === 'free')}
        {@const display = free ?? monthly ?? annual}
        {#if display}
          <article
            class="tier"
            class:selected={selected === display.plan_id ||
              (monthly && selected === monthly.plan_id) ||
              (annual && selected === annual.plan_id)}
            class:featured={t.tier === 'Estate'}
          >
            <h2>{t.tier}</h2>
            <div class="price">{display.display_price}</div>
            {#if monthly && annual}
              <p class="alt">
                or {annual.display_price}
                ({Math.round((1 - annual.price_usd_minor! / (monthly.price_usd_minor! * 12)) * 100)}% off)
              </p>
            {/if}
            <ul class="facts">
              <li><strong>{fmtBytes(display.storage_bytes)}</strong> storage</li>
              <li><strong>{yearsFromDays(display.retention_days)}</strong> retention</li>
              <li><strong>{yearsFromDays(display.scheduled_horizon_days)}</strong> scheduled horizon</li>
              <li>
                {display.max_vaults} Vault{display.max_vaults === 1 ? '' : 's'},
                {display.max_letters_per_vault} Letter{display.max_letters_per_vault === 1 ? '' : 's'} each
              </li>
              <li>{display.allowed_signals.length} signal source{display.allowed_signals.length === 1 ? '' : 's'}</li>
              {#if display.sms_recipients_allowed}<li>SMS recipients</li>{/if}
              {#if display.priority_support}<li>Priority support</li>{/if}
              {#if display.concierge_dunning}<li>Concierge dunning</li>{/if}
              {#if display.published_audit}<li>Published Vault audit</li>{/if}
            </ul>
            {#if display.notes.length > 0}
              <ul class="notes">
                {#each display.notes as n (n)}<li>{n}</li>{/each}
              </ul>
            {/if}
            <div class="actions">
              {#if monthly && annual}
                <label class="cad">
                  <input
                    type="radio"
                    name="cadence_{t.tier}"
                    checked={selected === monthly.plan_id}
                    onchange={() => (selected = monthly.plan_id)}
                  />
                  Monthly
                </label>
                <label class="cad">
                  <input
                    type="radio"
                    name="cadence_{t.tier}"
                    checked={selected === annual.plan_id}
                    onchange={() => (selected = annual.plan_id)}
                  />
                  Annual
                </label>
              {:else}
                <button
                  type="button"
                  class="select-btn"
                  class:active={selected === display.plan_id}
                  onclick={() => (selected = display.plan_id)}
                >
                  {selected === display.plan_id ? 'Selected' : 'Choose'}
                </button>
              {/if}
            </div>
          </article>
        {/if}
      {/each}
    </div>
  {/if}

  <Card eyebrow="HOW WE COMPARE" title="Against the closest products">
    <p class="dim">
      Paschal isn't the only product in this space. Here is the honest
      side-by-side. Strikethrough means we don't yet do it; we're explicit
      about what's missing.
    </p>
    <div class="compare-wrap">
      <table class="compare">
        <thead>
          <tr>
            <th>Feature</th>
            <th class="us">Paschal</th>
            <th>Bitwarden Emergency&nbsp;Access</th>
            <th>Cipherwill</th>
            <th>GoodTrust</th>
            <th>Vault12</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>Multi-signal dead-man's-switch</td>
            <td class="us">Yes — Heartbeat + Buddies + iCloud + financial + more</td>
            <td>No — wait-period only</td>
            <td>Wait-period</td>
            <td>Wait-period</td>
            <td>Wait-period</td>
          </tr>
          <tr>
            <td>Scheduled-release Letters (future date)</td>
            <td class="us">Yes — 1y / 3y / 10y / 25y by plan</td>
            <td>No</td>
            <td>No</td>
            <td>Limited</td>
            <td>No</td>
          </tr>
          <tr>
            <td>Cooling-off cancel before release</td>
            <td class="us">Yes — configurable 14d default</td>
            <td>Yes (wait-period)</td>
            <td>Yes</td>
            <td>Yes</td>
            <td>Yes</td>
          </tr>
          <tr>
            <td>Drill mode (rehearsal)</td>
            <td class="us">Yes — exercise plumbing without releasing</td>
            <td>No</td>
            <td>No</td>
            <td>No</td>
            <td>No</td>
          </tr>
          <tr>
            <td>Operator-blind crypto (Zero-Knowledge tier)</td>
            <td class="us"><em>Designed; ships Q4 2026 after audit</em></td>
            <td>End-to-end</td>
            <td>End-to-end</td>
            <td>Operator can see</td>
            <td>Threshold Shamir (operator-blind)</td>
          </tr>
          <tr>
            <td>Post-cancel retention</td>
            <td class="us">Up to <strong>25 years</strong> (Legacy plan)</td>
            <td>n/a — tied to password manager</td>
            <td>Unclear</td>
            <td>~2 years</td>
            <td>Tied to subscription</td>
          </tr>
          <tr>
            <td>Free tier</td>
            <td class="us">Yes — Draft (1 Letter, 1 Vault, Heartbeat only)</td>
            <td>Yes (with Bitwarden free)</td>
            <td>Trial only</td>
            <td>Limited free</td>
            <td>No</td>
          </tr>
          <tr>
            <td>Self-hostable</td>
            <td class="us">Yes — AGPL, Docker Compose, one-screen bootstrap</td>
            <td>Yes (self-host Bitwarden)</td>
            <td>No</td>
            <td>No</td>
            <td>No</td>
          </tr>
          <tr>
            <td>Independent crypto audit</td>
            <td class="us"><em>Scheduled Q3 2026, published in full</em></td>
            <td>Yes — annual</td>
            <td>Unclear</td>
            <td>Unclear</td>
            <td>Yes</td>
          </tr>
          <tr>
            <td>Transparency log / dead-company protocol</td>
            <td class="us">Yes — escrowed archive + read-only release server</td>
            <td>No</td>
            <td>No</td>
            <td>No</td>
            <td>Partial</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p class="dim small">
      Last updated 2026-05. Names and trademarks belong to their respective owners.
      Reviewed honestly — if you spot something wrong, email
      <a href="mailto:hello@paschal.app">hello@paschal.app</a> and we'll correct it.
    </p>
  </Card>

  <Card eyebrow="SIGN IN" title="Continue with your email">
    <form onsubmit={submit}>
      <p class="dim">
        For the MVP, sign-in is the same call as sign-up. Repeated entries with
        the same email return your existing account.
      </p>
      <Field
        bind:value={email}
        label="Email"
        name="email"
        type="email"
        autocomplete="email"
        required
      />
      <p class="dim small">
        Selected plan: <strong>{selectedPlan?.tier ?? selected}</strong>
        — {selectedPlan?.display_price ?? ''}
      </p>

      <label class="tos-row">
        <input type="checkbox" bind:checked={tosAccepted} />
        I accept the <a href="{base}/docs#service-continuity" target="_blank" rel="noopener">Terms of Service</a>.
        I understand that if Paschal can no longer operate, my data will be released to me or my nominated Co-Steward as a secure export.
      </label>

      {#if error}<Banner kind="warn">{error}</Banner>{/if}

      <div class="row">
        <Button type="submit" disabled={submitting || !email || !tosAccepted}>
          {submitting ? 'Connecting…' : 'Continue'}
        </Button>
        <a class="text-link" href="{base}/">Back</a>
      </div>
    </form>

    <div class="passkey-signin">
      <p class="dim small">Already registered a passkey on this account?</p>
      <Button variant="secondary" onclick={signInWithPasskey} disabled={passkeySubmitting || !email}>
        {passkeySubmitting ? 'Waiting for passkey…' : 'Sign in with a passkey'}
      </Button>
    </div>
  </Card>
</div>

<style>
  .head { text-align: center; margin-bottom: var(--sp-5); }
  h1 { font-size: var(--size-display-2); margin: var(--sp-1) 0; }
  .lede {
    font-family: var(--font-display);
    color: var(--slate);
    font-size: 22px;
    max-width: 640px;
    margin: var(--sp-2) auto 0;
  }

  .ladder {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--sp-3);
    margin-bottom: var(--sp-5);
  }
  @media (max-width: 1024px) { .ladder { grid-template-columns: repeat(2, 1fr); } }
  @media (max-width: 600px)  { .ladder { grid-template-columns: 1fr; } }

  .tier {
    background: var(--vellum);
    border: 1px solid var(--mist);
    padding: var(--sp-3);
    display: flex;
    flex-direction: column;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }
  .tier.featured { border: 2px solid var(--ink); }
  .tier.selected {
    border-color: var(--burgundy);
    box-shadow: 0 0 0 2px var(--burgundy);
  }
  .tier h2 { font-size: 28px; margin: 0 0 var(--sp-1); }
  .price { font-family: var(--font-display); font-size: 22px; color: var(--ink); margin-bottom: 4px; }
  .alt { color: var(--slate); font-size: var(--size-caption); margin: 0 0 var(--sp-2); }
  .facts {
    list-style: none;
    padding: 0;
    margin: var(--sp-2) 0;
    font-size: var(--size-body-2);
  }
  .facts li { padding: 4px 0; border-bottom: var(--rule); }
  .notes {
    margin: var(--sp-2) 0;
    padding-left: 18px;
    font-size: var(--size-caption);
    color: var(--slate);
  }
  .actions {
    margin-top: auto;
    padding-top: var(--sp-2);
    display: flex;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }
  .cad {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--size-body-2);
    cursor: pointer;
  }
  .select-btn {
    background: transparent;
    border: 1px solid var(--ink);
    color: var(--ink);
    font-family: inherit;
    font-size: var(--size-body-2);
    font-weight: 600;
    padding: 8px 16px;
    cursor: pointer;
  }
  .select-btn.active { background: var(--ink); color: var(--parchment); }
  .tos-row {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    font-size: var(--size-body-2);
    color: var(--slate);
    margin-bottom: var(--sp-3);
    cursor: pointer;
    line-height: 1.5;
  }
  .tos-row input { margin-top: 3px; flex-shrink: 0; }
  .tos-row a { color: var(--ink); }
  .row { display: flex; gap: var(--sp-3); align-items: center; margin-top: var(--sp-2); }
  .passkey-signin { margin-top: var(--sp-3); border-top: var(--rule); padding-top: var(--sp-3); }
  .passkey-signin .small { margin-bottom: var(--sp-2); }
  .text-link { color: var(--slate); text-decoration: underline; text-underline-offset: 4px; }
  .text-link:hover { color: var(--ink); }
  .small { font-size: var(--size-caption); }

  .audit {
    margin-top: var(--sp-2);
    font-size: var(--size-caption);
    color: var(--slate);
  }
  .audit a { color: var(--ink); text-underline-offset: 4px; }

  .compare-wrap { overflow-x: auto; margin-top: var(--sp-2); }
  table.compare {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--size-body-2);
    min-width: 720px;
  }
  table.compare th,
  table.compare td {
    padding: 10px 12px;
    border-bottom: var(--rule);
    text-align: left;
    vertical-align: top;
  }
  table.compare thead th {
    font-family: var(--font-display);
    color: var(--slate);
    font-weight: 600;
    font-size: var(--size-caption);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  table.compare th.us,
  table.compare td.us {
    background: rgba(70, 21, 42, 0.04);
  }
  table.compare th.us { color: var(--burgundy); }
  table.compare td.us { font-weight: 500; color: var(--ink); }
  table.compare em { color: var(--slate); font-style: italic; }
</style>
