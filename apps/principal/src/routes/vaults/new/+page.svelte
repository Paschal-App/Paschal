<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import {
    createVault,
    getSubscription,
    listPlans,
    STORAGE_REGIONS,
    ApiError
  } from '$lib/api';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let name = $state('');
  let coolingOffSeconds = $state('1209600'); // 14 days
  let storageRegion = $state(STORAGE_REGIONS[0]?.code ?? '');
  let multiRegion = $state(false);
  let submitting = $state(false);
  let error = $state<string | null>(null);

  // Estate+/Legacy plans may choose where each Vault's attachments are stored.
  $effect(() => {
    (async () => {
      try {
        const [sub, plans] = await Promise.all([getSubscription(), listPlans()]);
        multiRegion = plans.find((p) => p.plan_id === sub.plan_id)?.multi_region ?? false;
      } catch {
        multiRegion = false;
      }
    })();
  });

  async function submit(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    try {
      const v = await createVault({
        name,
        cooling_off_seconds: parseInt(coolingOffSeconds, 10),
        ...(multiRegion ? { storage_region: storageRegion } : {})
      });
      goto(`${base}/vaults/${v.id}`);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="page narrow">
  <Card eyebrow="NEW VAULT" title="Create a Vault">
    <form onsubmit={submit}>
      <p class="dim">
        A Vault is a container for Letters. You can have several, each with
        its own cooling-off and signal policy.
      </p>

      <Field
        bind:value={name}
        label="Name"
        name="name"
        required
        placeholder="Family, Work, Source list — whatever you call it"
        help="For you only. Recipients never see this."
      />

      <Field
        bind:value={coolingOffSeconds}
        label="Cooling-off window (seconds)"
        name="cooling_off_seconds"
        type="number"
        help="The default is 14 days (1,209,600 s). Set lower while you experiment with drills."
      />

      {#if multiRegion}
        <label class="region">
          <span class="region-label">Storage region for attachments</span>
          <select bind:value={storageRegion} name="storage_region">
            {#each STORAGE_REGIONS as r}
              <option value={r.code}>{r.label}</option>
            {/each}
          </select>
          <span class="region-help dim">
            Where this Vault's sealed files are stored. Letter text and all
            metadata stay in our primary region. You can move a Vault's files
            between regions later.
          </span>
        </label>
      {/if}

      {#if error}
        <Banner kind="warn">{error}</Banner>
      {/if}

      <div class="row">
        <Button type="submit" disabled={submitting || !name.trim()}>
          {submitting ? 'Sealing…' : 'Create Vault'}
        </Button>
        <a class="text-link" href="{base}/dashboard">Cancel</a>
      </div>
    </form>
  </Card>
</div>

<style>
  .region { display: flex; flex-direction: column; gap: var(--sp-1); margin-top: var(--sp-3); }
  .region-label { font-weight: 600; }
  .region select {
    padding: var(--sp-2);
    border: 1px solid var(--rule-ink);
    border-radius: var(--radius-sm, 4px);
    background: var(--paper, #fff);
    color: var(--ink);
    font: inherit;
  }
  .region-help { font-size: 0.85rem; }
  .row { display: flex; gap: var(--sp-3); align-items: center; margin-top: var(--sp-2); }
  .text-link { color: var(--slate); text-decoration: underline; text-underline-offset: 4px; }
  .text-link:hover { color: var(--ink); }
</style>
