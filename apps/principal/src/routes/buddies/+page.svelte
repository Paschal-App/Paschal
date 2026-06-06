<script lang="ts">
  import { listBuddies, inviteBuddy, revokeBuddy, ApiError } from '$lib/api';
  import type { Buddy } from '$lib/types';
  import { fmtDate, fmtRelative } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let buddies = $state<Buddy[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let inviteEmail = $state('');
  let inviteName = $state('');
  let cadence = $state('90');
  let submitting = $state(false);
  let inviteResult = $state<{ buddy: Buddy; token?: string } | null>(null);

  async function load() {
    loading = true;
    try {
      buddies = await listBuddies();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  async function invite(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    try {
      const r = await inviteBuddy({
        email: inviteEmail,
        display_name: inviteName || undefined,
        prompt_cadence_days: parseInt(cadence, 10)
      });
      inviteResult = { buddy: r.buddy, token: r.confirmation_token_DEV_ONLY };
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
    if (!confirm('Revoke this Buddy? They will stop receiving prompts.')) return;
    try {
      await revokeBuddy(id);
      await load();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    }
  }

  $effect(() => {
    load();
  });
</script>

<div class="page narrow">
  <header class="page-head">
    <div>
      <div class="eyebrow">BUDDIES</div>
      <h1>Trusted contacts</h1>
    </div>
  </header>

  {#if error}<Banner kind="warn">{error}</Banner>{/if}

  <Card eyebrow="INVITE" title="Add a Buddy">
    {#if inviteResult}
      <Banner kind="ok">
        Buddy invited: <strong>{inviteResult.buddy.email}</strong>.
        {#if inviteResult.token}
          <p style="margin-top: var(--sp-2)" class="dim small">
            In development the confirmation link is here so you can complete
            the flow without an email server:
          </p>
          <code class="token">{inviteResult.token}</code>
        {/if}
      </Banner>
    {/if}

    <form onsubmit={invite}>
      <p class="dim">
        A Buddy is an informal trusted contact who'll occasionally confirm
        you're well. <strong>At least two Buddies must respond</strong> for
        their input to count toward the release decision.
      </p>

      <Field bind:value={inviteEmail} label="Email" name="email" type="email" required />
      <Field bind:value={inviteName} label="Display name (optional)" name="display_name" />
      <Field
        bind:value={cadence}
        label="Prompt cadence (days)"
        name="prompt_cadence_days"
        type="number"
        help="How often we'll ask the Buddy whether you're well. Default 90."
      />

      <div class="row">
        <Button type="submit" disabled={submitting || !inviteEmail}>
          {submitting ? 'Inviting…' : 'Invite Buddy'}
        </Button>
      </div>
    </form>
  </Card>

  <Card eyebrow="ROSTER" title="Your Buddies">
    {#if loading}
      <p class="dim">Loading…</p>
    {:else if buddies.length === 0}
      <p>No Buddies yet.</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>Email</th>
            <th>Status</th>
            <th>Last response</th>
            <th>Cadence</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each buddies as b (b.id)}
            <tr class:revoked={b.revoked}>
              <td>{b.email}</td>
              <td>
                {#if b.revoked}
                  <span class="dim">Revoked</span>
                {:else if b.confirmed}
                  <span class="ok">Confirmed</span>
                {:else}
                  <span class="pending">Pending</span>
                {/if}
              </td>
              <td>
                {#if b.last_response}
                  <strong>{b.last_response}</strong>
                  <span class="dim small"> · {fmtRelative(b.last_response_at)}</span>
                {:else}
                  <span class="dim">—</span>
                {/if}
              </td>
              <td>{b.prompt_cadence_days}d</td>
              <td class="right">
                {#if !b.revoked}
                  <button type="button" class="text-link" onclick={() => revoke(b.id)}>
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
  .page-head { margin-bottom: var(--sp-4); }
  h1 { margin: 4px 0 0; font-size: var(--size-display-2); }
  .row { display: flex; gap: var(--sp-3); align-items: center; margin-top: var(--sp-2); }
  .right { text-align: right; }
  .ok { color: var(--verdigris); font-weight: 600; }
  .pending { color: var(--amber); }
  .revoked { opacity: 0.6; }
  .text-link {
    background: none;
    border: none;
    color: var(--burgundy);
    text-decoration: underline;
    text-underline-offset: 4px;
    cursor: pointer;
    font-size: var(--size-body-2);
    padding: 0;
  }
  .text-link:hover { color: var(--burgundy-deep); }
  .small { font-size: var(--size-caption); }
  .token {
    display: block;
    margin-top: 4px;
    padding: 6px 8px;
    background: var(--bone);
    border: 1px solid var(--mist);
    font-family: var(--font-mono);
    font-size: var(--size-caption);
    word-break: break-all;
  }
</style>
