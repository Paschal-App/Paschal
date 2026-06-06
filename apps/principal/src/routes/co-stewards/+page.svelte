<script lang="ts">
  import { listCoStewards, inviteCoSteward, revokeCoSteward, getSubscription, ApiError, type CoSteward } from '$lib/api';
  import { fmtDate, fmtRelative } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let stewards = $state<CoSteward[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let info = $state<string | null>(null);

  let inviteEmail = $state('');
  let inviteName = $state('');
  let submitting = $state(false);
  let lastConfirmationUrl = $state<string | null>(null);
  let planTier = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      const [list, sub] = await Promise.all([listCoStewards(), getSubscription()]);
      stewards = list;
      // Just to drive copy — the server enforces the quota.
      planTier = sub.plan_id;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { load(); });

  async function invite(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    info = null;
    try {
      const resp = await inviteCoSteward({
        email: inviteEmail,
        display_name: inviteName || undefined
      });
      // Skeleton convenience: surface the confirmation URL so we can
      // hand it to the invitee. In production this is emailed and the
      // *_DEV_ONLY field is gone.
      const token = resp.confirmation_token_DEV_ONLY;
      lastConfirmationUrl = `${location.origin}/app/co-steward/confirm?token=${encodeURIComponent(token)}`;
      info = `Invited ${resp.co_steward.email}.`;
      inviteEmail = '';
      inviteName = '';
      await load();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      submitting = false;
    }
  }

  async function revoke(id: string) {
    if (!confirm('Revoke this Co-Steward? They lose access immediately.')) return;
    error = null;
    try {
      await revokeCoSteward(id);
      await load();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    }
  }
</script>

<div class="page">
  <header class="head">
    <div class="eyebrow">SHARING</div>
    <h1>Co-Stewards</h1>
    <p class="lede">
      A Co-Steward is someone you trust to <strong>see</strong> the state of your Vaults —
      last heartbeat, signal strength, scheduled releases — without ever being able
      to read Letter contents or change anything.
    </p>
  </header>

  {#if error}<Banner kind="warn">{error}</Banner>{/if}
  {#if info}<Banner kind="ok">{info}</Banner>{/if}

  {#if lastConfirmationUrl}
    <Card eyebrow="DEV ONLY" title="Share this confirmation URL">
      <p class="dim">
        In production this is emailed automatically. While in dev, copy this URL and
        send it to the invitee yourself:
      </p>
      <pre>{lastConfirmationUrl}</pre>
    </Card>
  {/if}

  <Card eyebrow="INVITE" title="Add a Co-Steward">
    <p class="dim">
      They'll get an email with a one-time URL. The first time they open it, they
      choose a passphrase and accept the role. From then on they sign in at
      <code>/app/co-steward/sign-in</code>.
    </p>
    <form onsubmit={invite}>
      <Field
        bind:value={inviteEmail}
        label="Email"
        name="email"
        type="email"
        required
        help="Where the confirmation email goes."
      />
      <Field
        bind:value={inviteName}
        label="Display name (optional)"
        name="display_name"
        help="So you can tell them apart on this page later."
      />
      <Button type="submit" disabled={submitting || !inviteEmail}>
        {submitting ? 'Inviting…' : 'Send invitation'}
      </Button>
    </form>
  </Card>

  <Card eyebrow="ROSTER" title="Current Co-Stewards">
    {#if loading}
      <p class="dim">Loading…</p>
    {:else if stewards.length === 0}
      <p class="dim">No Co-Stewards yet.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Email</th>
            <th>Status</th>
            <th>Last viewed</th>
            <th>Invited</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each stewards as c (c.id)}
            <tr>
              <td>
                {#if c.display_name}<strong>{c.display_name}</strong><br />{/if}
                {c.email}
              </td>
              <td>
                {#if c.revoked}
                  <span class="badge revoked">Revoked</span>
                {:else if c.confirmed}
                  <span class="badge ok">Confirmed</span>
                {:else}
                  <span class="badge pending">Pending</span>
                {/if}
              </td>
              <td>{c.last_viewed_at ? fmtRelative(c.last_viewed_at) : '—'}</td>
              <td>{fmtDate(c.created_at)}</td>
              <td class="right">
                {#if !c.revoked}
                  <button type="button" class="link-btn" onclick={() => revoke(c.id)}>
                    Revoke
                  </button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>

<style>
  .head { margin-bottom: var(--sp-4); }
  h1 { font-size: var(--size-display-2); margin: 4px 0; }
  .lede {
    font-family: var(--font-display);
    color: var(--slate);
    font-size: 18px;
    max-width: 640px;
    margin: var(--sp-2) 0 0;
  }
  pre {
    background: var(--bone);
    padding: var(--sp-2);
    font-size: var(--size-caption);
    overflow-x: auto;
    word-break: break-all;
    white-space: pre-wrap;
  }
  .right { text-align: right; }
  .link-btn {
    background: none;
    border: none;
    color: var(--burgundy);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 4px;
  }
  .badge {
    display: inline-block;
    padding: 2px 8px;
    font-size: var(--size-caption);
    font-family: var(--font-display);
  }
  .badge.ok { background: var(--bone); color: var(--ink); }
  .badge.pending { background: var(--mist); color: var(--slate); }
  .badge.revoked { background: var(--mist); color: var(--slate); text-decoration: line-through; }
</style>
