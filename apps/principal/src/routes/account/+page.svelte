<script lang="ts">
  import { goto } from '$app/navigation';
  import {
    getSubscription,
    requestAccountDeletion,
    cancelAccountDeletion,
    ApiError
  } from '$lib/api';
  import type { Subscription } from '$lib/types';
  import { fmtDate, fmtRelative } from '$lib/format';
  import { clear } from '$lib/session';
  import Card from '$lib/components/Card.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let subscription = $state<Subscription | null>(null);
  let loading = $state(true);
  let action = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    try {
      subscription = await getSubscription();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  async function requestDelete() {
    if (!confirm(
      'Delete your account? This is permanent after 30 days. Every Vault, Letter, and Attachment will be cryptographically erased.'
    )) return;
    action = 'delete';
    try {
      const r = await requestAccountDeletion();
      notice = `Deletion scheduled for ${fmtDate(r.deletion_scheduled_for)}. You can cancel this until then.`;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      action = null;
    }
  }

  async function undelete() {
    action = 'undelete';
    try {
      await cancelAccountDeletion();
      notice = 'Deletion cancelled. Your account is safe.';
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      action = null;
    }
  }

  $effect(() => { load(); });
</script>

<div class="page narrow">
  <header class="page-head">
    <div>
      <div class="eyebrow">ACCOUNT</div>
      <h1>Subscription &amp; data</h1>
    </div>
  </header>

  {#if notice}<Banner kind="ok">{notice}</Banner>{/if}
  {#if error}<Banner kind="warn">{error}</Banner>{/if}

  <Card eyebrow="SUBSCRIPTION" title="Your Subscription">
    {#if loading}
      <p class="dim">Loading…</p>
    {:else if !subscription}
      <p>No Subscription found.</p>
    {:else}
      <dl>
        <dt>Plan</dt>
        <dd><strong>{subscription.plan_id}</strong></dd>
        <dt>State</dt>
        <dd>{subscription.state}</dd>
        <dt>Started</dt>
        <dd>{fmtDate(subscription.started_at)}</dd>
        {#if subscription.trial_end_at}
          <dt>Trial ends</dt>
          <dd>{fmtDate(subscription.trial_end_at)} ({fmtRelative(subscription.trial_end_at)})</dd>
        {/if}
        {#if subscription.canceled_at}
          <dt>Cancelled</dt>
          <dd>{fmtDate(subscription.canceled_at)}</dd>
          <dt>Retention until</dt>
          <dd>{fmtDate(subscription.retention_until)}</dd>
        {/if}
      </dl>
      <p class="dim small" style="margin-top: var(--sp-2)">
        This is a self-hosted instance. Subscription state tracks data
        retention; all features are available regardless of state.
      </p>
    {/if}
  </Card>

  <Card eyebrow="DANGER ZONE" title="Delete your account">
    <p>
      Permanently destroys every Vault, Letter, and Attachment. The
      transparency log retains hashes only. There is a 30-day cool-off
      during which you can reverse the request.
    </p>
    <div class="row" style="margin-top: var(--sp-3)">
      <Button variant="danger" onclick={requestDelete} disabled={action !== null}>
        {action === 'delete' ? 'Requesting…' : 'Request deletion'}
      </Button>
      <button type="button" class="text-link" onclick={undelete} disabled={action !== null}>
        Cancel a pending deletion
      </button>
    </div>
  </Card>
</div>

<style>
  .page-head { margin-bottom: var(--sp-4); }
  h1 { margin: 4px 0 0; font-size: var(--size-display-2); }
  dl { display: grid; grid-template-columns: 180px 1fr; gap: 4px var(--sp-3); margin: 0; }
  dt { font-size: var(--size-caption); color: var(--slate); }
  dd { margin: 0; }
  .row { display: flex; gap: var(--sp-3); align-items: center; }
  .small { font-size: var(--size-caption); }
  .text-link {
    background: none;
    border: none;
    color: var(--slate);
    text-decoration: underline;
    text-underline-offset: 4px;
    cursor: pointer;
    padding: 0;
  }
  .text-link:hover { color: var(--ink); }
  .text-link:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
