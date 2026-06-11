<script lang="ts">
  import { base } from '$app/paths';
  import { listVaults, getSubscription, heartbeat, listBuddies, getUsage, getBankSignalStatus, enrolBankSignal, revokeBankSignal, getDuress, armDuress, revokeDuress, ApiError, type Usage, type BankSignalStatus, type DuressStatus } from '$lib/api';
  import { fmtDate, fmtRelative, fmtBytes } from '$lib/format';
  import type { Vault, Subscription, Buddy } from '$lib/types';
  import Card from '$lib/components/Card.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';
  import StatusBadge from '$lib/components/StatusBadge.svelte';
  import Field from '$lib/components/Field.svelte';
  import GettingStarted from '$lib/components/GettingStarted.svelte';

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
  let duress = $state<DuressStatus | null>(null);
  let duressEmail = $state('');
  let duressPanicMode = $state<'freeze' | 'release'>('freeze');
  let duressAction = $state<string | null>(null);

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
      duress = await getDuress().catch(() => null);
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

  async function armDuressHandler() {
    duressAction = 'arm';
    try {
      duress = await armDuress(duressEmail, duressPanicMode);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      duressAction = null;
    }
  }

  async function disarmDuressHandler() {
    if (!confirm('Disarm the duress signal? The panic URL stops working until you arm it again.')) return;
    duressAction = 'disarm';
    try {
      await revokeDuress();
      duress = null;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      duressAction = null;
    }
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
        return `Trial period — ends ${fmtRelative(subscription.trial_end_at)}.`;
      case 'ACTIVE':
        return 'Active.';
      case 'PAST_DUE':
        return 'Past due.';
      case 'CANCELED':
        return `Cancelled. Vaults remain releasable until ${fmtDate(subscription.retention_until)}.`;
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

  <GettingStarted {vaults} {buddies} {bankSignal} {loading} />

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
              {usage.vaults_used} of {usage.vaults_quota === 4294967295 ? '∞' : usage.vaults_quota}
            </span>
          </div>
          <div class="bar" aria-hidden="true">
            <div
              class="fill"
              class:warn={usage.vaults_quota < 4294967295 && usage.vaults_used / usage.vaults_quota >= 0.8}
              class:full={usage.vaults_quota < 4294967295 && usage.vaults_used >= usage.vaults_quota}
              style="width: {usage.vaults_quota < 4294967295 ? Math.min((usage.vaults_used / usage.vaults_quota) * 100, 100) : 0}%"
            ></div>
          </div>
        </div>
      </div>

      <p class="meta">
        Each Vault holds up to <strong>{usage.letters_quota_per_vault === 4294967295 ? '∞' : usage.letters_quota_per_vault}</strong> Letter{usage.letters_quota_per_vault === 1 ? '' : 's'}.
        Retention after cancel: <strong>{Math.round(usage.retention_days / 365)} year{usage.retention_days >= 730 ? 's' : ''}</strong>.
        Scheduled-release horizon: <strong>{Math.round(usage.scheduled_horizon_days / 365)} year{usage.scheduled_horizon_days >= 730 ? 's' : ''}</strong>.
      </p>

      {#if usage.storage_pct >= 80}
        <Banner kind="warn">
          You're at {Math.round(usage.storage_pct)}% of your storage quota.
          Remove older Letters or increase <code>MAX_UPLOAD_BYTES</code> in your deployment configuration.
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

  <Card eyebrow="SIGNALS" title="Proof-of-life" id="signals">
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

  <Card eyebrow="DURESS" title="Panic signal" id="duress">
    {#snippet children()}
      {#if duress?.armed}
        {#if duress.triggered_at}
          <Banner kind="warn">
            Duress is active — every Vault's release is frozen and your contact was alerted.
            Disarm once you are safe to resume normal operation.
          </Banner>
        {/if}
        <p class="dim small">
          A private URL is armed. A POST to it — from a gift-card purchase or any action you can
          take under coercion — silently freezes all releases and alerts your contact. Nothing
          visible changes on your device.
        </p>
        <label class="url-label">Your private duress URL</label>
        <div class="webhook-row">
          <code class="mono small">{duress.webhook_url}</code>
          <button type="button" class="copy-btn" onclick={() => copyWebhookUrl(duress!.webhook_url!)}>
            {copied ? 'Copied!' : 'Copy'}
          </button>
        </div>
        <details class="how-to">
          <summary>Connect it to a gift-card purchase</summary>
          <ol class="steps">
            <li>Pick a specific, plausible action — e.g. buying a $50 gift card of a set brand.</li>
            <li>Wire it to a POST to this URL: an Apple Shortcut on the store app's notification, or a Zapier / Make rule on that transaction.</li>
            <li>Under coercion, perform the action. The dead-man's switch freezes and your contact is alerted — quietly.</li>
          </ol>
        </details>
        <button type="button" class="text-link revoke-link" onclick={disarmDuressHandler} disabled={duressAction !== null}>
          Disarm duress signal
        </button>
      {:else}
        <p class="dim">No duress signal armed.</p>
        <p class="dim small" style="margin-bottom: var(--sp-2)">
          A covert panic switch. Arm a private URL and connect it to an action you can take under
          coercion (a set gift-card purchase). Triggering it silently <strong>freezes every Vault's
          release</strong> — so a captor can't force letters out — and discreetly alerts a contact.
        </p>
        <Field
          bind:value={duressEmail}
          label="Alert this contact (optional)"
          name="duress_email"
          placeholder="trusted@example.com"
          help="They get a discreet wellbeing-check message when you trigger duress."
        />
        <fieldset style="border:var(--rule);padding:var(--sp-2);margin:var(--sp-2) 0;display:flex;flex-direction:column;gap:var(--sp-1);">
          <legend style="font-size:var(--size-caption);color:var(--slate);text-transform:uppercase;letter-spacing:0.04em;padding:0 6px;">When triggered</legend>
          <label style="display:flex;gap:var(--sp-1);align-items:flex-start;font-size:var(--size-body-2);cursor:pointer;">
            <input type="radio" name="panic_mode" value="freeze" bind:group={duressPanicMode} style="margin-top:4px;flex-shrink:0;" />
            <span><strong>Freeze</strong> — pause every Vault's release while duress is active (default; for coercion).</span>
          </label>
          <label style="display:flex;gap:var(--sp-1);align-items:flex-start;font-size:var(--size-body-2);cursor:pointer;">
            <input type="radio" name="panic_mode" value="release" bind:group={duressPanicMode} style="margin-top:4px;flex-shrink:0;" />
            <span><strong>Release</strong> — immediately start cooling-off on all active Vaults (carry out your wishes without delay).</span>
          </label>
        </fieldset>
        <Button onclick={armDuressHandler} disabled={duressAction !== null}>
          {duressAction === 'arm' ? 'Arming…' : 'Arm duress signal'}
        </Button>
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

  .url-label { font-size: var(--size-caption); color: var(--slate); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 4px; display: block; }
  .how-to {
    margin-top: var(--sp-2);
    border: var(--rule);
    padding: var(--sp-2);
    font-size: var(--size-body-2);
  }
  .how-to summary {
    cursor: pointer;
    font-weight: 600;
    color: var(--ink);
    list-style: none;
  }
  .how-to summary::before { content: '▶ '; font-size: 10px; }
  .how-to[open] summary::before { content: '▼ '; }
  .steps { padding-left: var(--sp-3); margin: var(--sp-2) 0 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .steps li { color: var(--slate); line-height: 1.5; }
  .steps code { font-family: var(--font-mono, monospace); font-size: 0.9em; background: var(--mist); padding: 1px 4px; }
</style>
