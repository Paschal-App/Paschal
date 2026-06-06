<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { ApiError, type CoStewardDashboard } from '$lib/api';
  import { fmtDate, fmtRelative } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Banner from '$lib/components/Banner.svelte';
  import StatusBadge from '$lib/components/StatusBadge.svelte';

  let dashboard = $state<CoStewardDashboard | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    const stash = localStorage.getItem('paschal_co_steward_session');
    if (!stash) {
      goto(`${base}/co-steward/sign-in`);
      return;
    }
    const { token } = JSON.parse(stash);
    try {
      const r = await fetch('/v1/co-stewards/me/dashboard', {
        headers: { authorization: `Bearer ${token}` }
      });
      if (r.status === 401) {
        localStorage.removeItem('paschal_co_steward_session');
        goto(`${base}/co-steward/sign-in`);
        return;
      }
      if (!r.ok) {
        const body = await r.json().catch(() => ({}));
        throw new ApiError(body);
      }
      dashboard = await r.json();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  function signOut() {
    localStorage.removeItem('paschal_co_steward_session');
    goto(`${base}/co-steward/sign-in`);
  }

  function lettersInVault(vid: string): number {
    if (!dashboard) return 0;
    const r = dashboard.letter_counts.find((c) => c.vault_id === vid);
    return r?.letters ?? 0;
  }

  $effect(() => { load(); });
</script>

<div class="page">
  <header class="head">
    <div>
      <div class="eyebrow">CO-STEWARD VIEW</div>
      <h1>Read-only dashboard</h1>
      {#if dashboard}
        <p class="lede">
          For <strong>{dashboard.principal_email}</strong>. You can see state;
          you cannot read Letters or change anything.
        </p>
      {/if}
    </div>
    <button type="button" class="link-btn" onclick={signOut}>Sign out</button>
  </header>

  {#if error}<Banner kind="warn">{error}</Banner>{/if}
  {#if loading}<p class="dim">Loading…</p>{/if}

  {#if dashboard}
    <Card eyebrow="SUBSCRIPTION" title={dashboard.subscription.state}>
      <p class="dim">
        Plan: <strong>{dashboard.subscription.plan_id}</strong>
        &nbsp;·&nbsp; Started: {fmtDate(dashboard.subscription.started_at)}
      </p>
      {#if dashboard.subscription.state === 'CANCELED'}
        <Banner kind="warn">
          Cancelled. Vaults remain releasable until {fmtDate(
            dashboard.subscription.retention_until
          )}.
        </Banner>
      {/if}
    </Card>

    <Card eyebrow="VAULTS" title="Vaults under management">
      {#if dashboard.vaults.length === 0}
        <p class="dim">No Vaults.</p>
      {:else}
        <table>
          <thead>
            <tr>
              <th>State</th>
              <th>Name</th>
              <th>Last attestation</th>
              <th>Letters</th>
              <th>Cooling-off</th>
            </tr>
          </thead>
          <tbody>
            {#each dashboard.vaults as v (v.id)}
              <tr>
                <td><StatusBadge state={v.state} /></td>
                <td>{v.name}</td>
                <td>{fmtRelative(v.last_attestation_at)}</td>
                <td>{lettersInVault(v.id)}</td>
                <td>{v.cooling_off_seconds}s</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </Card>

    <Card eyebrow="BUDDIES" title="Active Buddies">
      <p>
        <strong>{dashboard.buddy_count}</strong> active Buddy{dashboard.buddy_count === 1 ? '' : 's'} configured.
      </p>
      <p class="dim small">
        Co-Stewards do not see Buddy details. The count is informational only.
      </p>
    </Card>
  {/if}
</div>

<style>
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: var(--sp-4);
  }
  h1 { font-size: var(--size-display-2); margin: 4px 0; }
  .lede {
    font-family: var(--font-display);
    color: var(--slate);
    font-size: 18px;
    max-width: 640px;
    margin: var(--sp-2) 0 0;
  }
  .link-btn {
    background: none;
    border: none;
    color: var(--slate);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 4px;
  }
  .small { font-size: var(--size-caption); }
</style>
