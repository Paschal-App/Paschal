<script lang="ts">
  import { base } from '$app/paths';
  import { listVaults, getSubscription, heartbeat, listBuddies, getUsage, buyStorageAddon, getBankSignalStatus, enrolBankSignal, revokeBankSignal, ApiError, type Usage, type BankSignalStatus } from '$lib/api';
  import { fmtDate, fmtRelative, fmtBytes } from '$lib/format';
  import type { Vault, Subscription, Buddy } from '$lib/types';
  import Card from '$lib/components/Card.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';
  import StatusBadge from '$lib/components/StatusBadge.svelte';

  let purchasing = $state(false);
  async function purchaseAddon(bundle: '1gb' | '5gb' | '10gb') {
    purchasing = true;
    try {
      const r = await buyStorageAddon(bundle);
      if (r.stub) {
        alert(
          `Stripe billing isn't configured yet. In production this would redirect to:\n${r.checkout_url}\n\nMeanwhile, the storage add-on can be granted manually by the operator.`
        );
      } else {
        location.href = r.checkout_url;
      }
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      purchasing = false;
    }
  }

  let vaults = $state<Vault[]>([]);
  let subscription = $state<Subscription | null>(null);
  let buddies = $state<Buddy[]>([]);
  let usage = $state<Usage | null>(null);
  let bankSignal = $state<BankSignalStatus | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let beating = $state(false);
  let beatedAt = $state<string | null>(null);
  let bankAction = $state<string | null>(null);
  let copied = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      const [v, s, b, u] = await Promise.all([
        listVaults(),
        getSubscription(),
        listBuddies(),
        getUsage()
      ]);
      vaults = v;
      subscription = s;
      buddies = b;
      usage = u;
      bankSignal = await getBankSignalStatus().catch(() => null);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  async function enrolBankSignalHandler() {
    bankAction = 'enrol';
    try {
      bankSignal = await enrolBankSignal();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      bankAction = null;
    }
  }

  async function revokeBankSignalHandler() {
    if (!confirm('Revoke bank activity webhook? Any automations using this URL will stop sending proof-of-life signals.')) return;
    bankAction = 'revoke';
    try {
      await revokeBankSignal();
      bankSignal = null;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      bankAction = null;
    }
  }

  async function copyWebhookUrl(url: string) {
    await navigator.clipboard.writeText(url);
    copied = true;
    setTimeout(() => { copied = false; }, 2000);
  }

  async function sendHeartbeat() {
    beating = true;
    try {
      const r = await heartbeat();
      beatedAt = r.received_at;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      beating = false;
    }
  }

  $effect(() => {
    load();
  });

  const subDescription = $derived.by(() => {
    if (!subscription) return '';
    switch (subscription.state) {
      case 'TRIALING':
        return `Trial — ends ${fmtRelative(subscription.trial_end_at)}.`;
      case 'ACTIVE':
        return `Active — renews ${fmtRelative(subscription.current_period_end)}.`;
      case 'PAST_DUE':
        return 'Past due — update payment to keep your Vaults authorable.';
      case 'CANCELED':
        return `Cancelled. Vaults remain releasable until ${fmtDate(
          subscription.retention_until
        )}.`;
      case 'EXPIRED':
        return 'Expired. Data is scheduled for erasure.';
      case 'DELETED':
        return 'Account deleted.';
    }
  });
</script>

<div class="page">
  <header class="page-head">
    <div>
      <div class="eyebrow">YOUR ACCOUNT</div>
      <h1>Dashboard</h1>
    </div>
    <div class="actions">
      <Button onclick={sendHeartbeat} disabled={beating}>
        {beating ? 'Sending…' : 'Heartbeat'}
      </Button>
      <Button variant="secondary" href="{base}/vaults/new">New Vault</Button>
    </div>
  </header>

  {#if beatedAt}
    <Banner kind="ok">Heartbeat recorded at {fmtDate(beatedAt)}.</Banner>
  {/if}
  {#if error}
    <Banner kind="warn">{error}</Banner>
  {/if}

  <Card eyebrow="SUBSCRIPTION" title={subscription?.state ?? '…'}>
    <p class="dim">{subDescription}</p>
    <p class="meta">
      Plan: <strong>{subscription?.plan_id ?? '—'}</strong>
      &nbsp;·&nbsp; Started: {fmtDate(subscription?.started_at)}
    </p>
  </Card>

  {#if usage}
    <Card eyebrow="USAGE" title="What you're using">
      <div class="meters">
        <div class="meter">
          <div class="row label">
            <span>Storage</span>
            <span class="dim">
              {fmtBytes(usage.storage_used_bytes)} of {fmtBytes(usage.storage_quota_bytes)}
            </span>
          </div>
          <div class="bar" aria-hidden="true">
            <div
              class="fill"
              class:warn={usage.storage_pct >= 80}
              class:full={usage.storage_pct >= 100}
              style="width: {Math.min(usage.storage_pct, 100)}%"
            ></div>
          </div>
        </div>

        <div class="meter">
          <div class="row label">
            <span>Vaults</span>
            <span class="dim">
              {usage.vaults_used} of {usage.vaults_quota}
            </span>
          </div>
          <div class="bar" aria-hidden="true">
            <div
              class="fill"
              class:warn={usage.vaults_used / usage.vaults_quota >= 0.8}
              class:full={usage.vaults_used >= usage.vaults_quota}
              style="width: {Math.min((usage.vaults_used / usage.vaults_quota) * 100, 100)}%"
            ></div>
          </div>
        </div>
      </div>

      <p class="meta">
        Each Vault holds up to <strong>{usage.letters_quota_per_vault}</strong> Letter{usage.letters_quota_per_vault === 1 ? '' : 's'}.
        Retention after cancel: <strong>{Math.round(usage.retention_days / 365)} year{usage.retention_days >= 730 ? 's' : ''}</strong>.
        Scheduled-release horizon: <strong>{Math.round(usage.scheduled_horizon_days / 365)} year{usage.scheduled_horizon_days >= 730 ? 's' : ''}</strong>.
        {#if usage.extra_storage_bytes > 0}
          <br />
          Storage add-on: <strong>{fmtBytes(usage.extra_storage_bytes)}</strong> on top of the
          plan base ({fmtBytes(usage.plan_base_storage_bytes)}).
        {/if}
      </p>

      {#if usage.storage_pct >= 80}
        <Banner kind="warn">
          You're at {Math.round(usage.storage_pct)}% of your storage quota.
          Consider <a href="{base}/account">upgrading your plan</a>, removing older Letters,
          or buying a storage add-on:
          <div class="addons">
            <button type="button" disabled={purchasing} onclick={() => purchaseAddon('1gb')}>
              +1 GB&nbsp;·&nbsp;$0.50/mo
            </button>
            <button type="button" disabled={purchasing} onclick={() => purchaseAddon('5gb')}>
              +5 GB&nbsp;·&nbsp;$2.50/mo
            </button>
            <button type="button" disabled={purchasing} onclick={() => purchaseAddon('10gb')}>
              +10 GB&nbsp;·&nbsp;$5.00/mo
            </button>
          </div>
        </Banner>
      {/if}
    </Card>
  {/if}

  <Card eyebrow="VAULTS" title="Your Vaults">
    {#if loading}
      <p class="dim">Loading…</p>
    {:else if vaults.length === 0}
      <p>No Vaults yet.</p>
      <p class="dim">Begin with <a href="{base}/vaults/new">a Family Vault</a> — one container per purpose.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>State</th>
            <th>Name</th>
            <th>Last attestation</th>
            <th>Cooling-off</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each vaults as v (v.id)}
            <tr>
              <td>
                <StatusBadge state={v.state} />
                <span class="pill enc" title={v.tier === 'ZERO_KNOWLEDGE' ? 'Keys never leave your device' : 'Encrypted at rest; operator holds key'}>
                  {v.tier === 'ZERO_KNOWLEDGE' ? 'ZK' : 'E2E'}
                </span>
              </td>
              <td>{v.name}</td>
              <td>{fmtRelative(v.last_attestation_at)}</td>
              <td>{v.cooling_off_seconds}s</td>
              <td class="right"><a href="{base}/vaults/{v.id}">Open →</a></td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>

  <Card eyebrow="BUDDIES" title="Your Buddies">
    {#if loading}
      <p class="dim">Loading…</p>
    {:else if buddies.length === 0}
      <p>No Buddies yet.</p>
      <p class="dim">
        Buddies are informal trusted contacts who can confirm you're well.
        At least two are needed to contribute meaningfully to a release decision.
      </p>
      <p><a href="{base}/buddies">Invite a Buddy →</a></p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Email</th>
            <th>Status</th>
            <th>Last response</th>
          </tr>
        </thead>
        <tbody>
          {#each buddies as b (b.id)}
            <tr>
              <td>{b.email}</td>
              <td>{b.confirmed ? 'Confirmed' : 'Pending'}</td>
              <td>{b.last_response ?? '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      <p style="margin-top: var(--sp-2)"><a href="{base}/buddies">Manage Buddies →</a></p>
    {/if}
  </Card>

  <Card eyebrow="SIGNALS" title="Proof-of-life">
    {#snippet children()}
      {#if bankSignal}
        <div class="signal-row">
          <div class="signal-header">
            <span class="pip active"></span>
            <span>Bank activity webhook</span>
            <span class="dim small">
              Last ping: {bankSignal.last_webhook_at ? fmtRelative(bankSignal.last_webhook_at) : 'never'}
            </span>
          </div>
          <div class="webhook-row">
            <code class="mono small">{bankSignal.webhook_url}</code>
            <button type="button" class="copy-btn" onclick={() => copyWebhookUrl(bankSignal!.webhook_url)}>
              {copied ? 'Copied!' : 'Copy'}
            </button>
          </div>
          <p class="dim small" style="margin-top: var(--sp-1)">
            POST this URL from YNAB webhooks, a bank automation tool, or an Apple Shortcut.
            Any POST counts as proof of activity — silence is treated as absence.
          </p>
          <button type="button" class="text-link revoke-link" onclick={revokeBankSignalHandler} disabled={bankAction !== null}>
            Revoke
          </button>
        </div>
      {:else}
        <p class="dim">No bank activity signal enrolled.</p>
        <Button onclick={enrolBankSignalHandler} disabled={bankAction !== null}>
          {bankAction === 'enrol' ? 'Enrolling…' : 'Set up bank activity webhook'}
        </Button>
        <p class="dim small" style="margin-top: var(--sp-2)">
          Generates a private webhook URL. Paste it into YNAB, your bank's automation,
          or an Apple Shortcut — any HTTP POST counts as proof of activity.
        </p>
      {/if}
    {/snippet}
  </Card>
</div>

<style>
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--sp-4);
  }
  .actions { display: flex; gap: var(--sp-2); }
  h1 { margin: 4px 0 0 0; font-size: var(--size-display-2); }
  .meta { font-size: var(--size-caption); color: var(--slate); margin-top: var(--sp-1); }
  .right { text-align: right; }
  td a { color: var(--ink); text-decoration: none; }
  td a:hover { color: var(--burgundy); text-decoration: underline; text-underline-offset: 4px; }

  .meters {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-bottom: var(--sp-2);
  }
  .meter .row.label {
    display: flex;
    justify-content: space-between;
    font-size: var(--size-body-2);
    margin-bottom: 6px;
  }
  .meter .bar {
    width: 100%;
    height: 8px;
    background: var(--mist);
    overflow: hidden;
  }
  .meter .fill {
    height: 100%;
    background: var(--ink);
    transition: width 240ms ease;
  }
  .meter .fill.warn { background: #b8860b; }
  .meter .fill.full { background: var(--burgundy); }
  .addons { display: flex; gap: var(--sp-2); flex-wrap: wrap; margin-top: var(--sp-2); }
  .addons button {
    background: transparent;
    border: 1px solid var(--ink);
    padding: 6px 12px;
    font-family: inherit;
    font-size: var(--size-body-2);
    cursor: pointer;
  }
  .addons button:hover:not(:disabled) {
    background: var(--ink);
    color: var(--parchment);
  }
  .addons button:disabled { opacity: 0.6; cursor: progress; }

  .pill.enc {
    display: inline-block;
    font-size: var(--size-micro);
    letter-spacing: 0.06em;
    padding: 1px 5px;
    border: 1px solid var(--verdigris);
    color: var(--verdigris);
    margin-left: 6px;
    vertical-align: middle;
    cursor: default;
  }
  .signal-row { display: flex; flex-direction: column; gap: var(--sp-1); }
  .signal-header { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .webhook-row { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .copy-btn {
    background: var(--ink);
    color: var(--parchment);
    border: none;
    padding: 4px 12px;
    font-family: inherit;
    font-size: var(--size-caption);
    cursor: pointer;
    flex-shrink: 0;
  }
  .copy-btn:hover { background: var(--burgundy); }
  .text-link { background: none; border: none; color: var(--slate); text-decoration: underline; text-underline-offset: 4px; cursor: pointer; padding: 0; font-family: inherit; font-size: var(--size-body-2); }
  .text-link:hover { color: var(--ink); }
  .text-link:disabled { opacity: 0.5; cursor: not-allowed; }
  .revoke-link { margin-top: var(--sp-1); }
  .small { font-size: var(--size-caption); }
</style>
