<script lang="ts">
  import { page } from '$app/state';
  import { base } from '$app/paths';
  import {
    getVault,
    listLetters,
    forceRelease,
    cancelRelease,
    runDrill,
    listVaultContacts,
    createVaultContact,
    deleteVaultContact,
    getSubscription,
    listPlans,
    moveVaultRegion,
    STORAGE_REGIONS,
    ApiError
  } from '$lib/api';
  import type { Letter, Vault, VaultContact } from '$lib/api';
  import { fmtDate, fmtRelative } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';
  import StatusBadge from '$lib/components/StatusBadge.svelte';

  const vaultId = $derived(page.params.id ?? '');

  let vault = $state<Vault | null>(null);
  let letters = $state<Letter[]>([]);
  let contacts = $state<VaultContact[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let action = $state<string | null>(null);
  let notice = $state<string | null>(null);

  // Contact form state
  let newName = $state('');
  let newEmail = $state('');
  let newBirthday = $state('');
  let newReleaseNote = $state('');
  let contactError = $state<string | null>(null);
  let savingContact = $state(false);

  // Storage region (Estate+/Legacy)
  let multiRegion = $state(false);
  let targetRegion = $state('');
  let movingRegion = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      const [v, ls, cs] = await Promise.all([
        getVault(vaultId),
        listLetters(vaultId),
        listVaultContacts(vaultId).catch(() => [] as VaultContact[])
      ]);
      vault = v;
      letters = ls;
      contacts = cs;
      targetRegion = v.storage_region;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (vaultId) load();
  });

  // Whether the principal's plan permits moving storage regions.
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

  async function doMoveRegion() {
    if (!vault || targetRegion === vault.storage_region) return;
    const label = STORAGE_REGIONS.find((r) => r.code === targetRegion)?.label ?? targetRegion;
    if (!confirm(`Move this Vault's sealed files to ${label}? Each attachment is re-encrypted in the new region and the old copy is removed.`)) return;
    movingRegion = true;
    notice = null;
    try {
      const v = await moveVaultRegion(vaultId, targetRegion);
      vault = v;
      targetRegion = v.storage_region;
      notice = `Storage region moved to ${v.storage_region_label}.`;
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      movingRegion = false;
    }
  }

  async function doDrill() {
    action = 'drill';
    notice = null;
    try {
      const r = await runDrill(vaultId);
      notice = `Drill started. Cooling-off ends ${fmtRelative(r.cooling_off_ends_at)}. Recipients receive a 'Rehearsal —' notice.`;
      setTimeout(load, 6000);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      action = null;
    }
  }

  async function doForceRelease() {
    if (!confirm('Force-release this Vault? This is the real thing — recipients will be notified after cooling-off elapses.')) return;
    action = 'release';
    notice = null;
    try {
      const r = await forceRelease(vaultId);
      notice = `Cooling-off started. Will release at ${fmtDate(r.cooling_off_ends_at)} unless cancelled.`;
      setTimeout(load, 2000);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      action = null;
    }
  }

  async function doCancel() {
    action = 'cancel';
    try {
      await cancelRelease(vaultId);
      notice = 'Release cancelled. Vault returned to Active.';
      await load();
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      action = null;
    }
  }

  async function saveContact(e: Event) {
    e.preventDefault();
    if (!newEmail.includes('@')) { contactError = 'Email looks invalid.'; return; }
    savingContact = true;
    contactError = null;
    try {
      const c = await createVaultContact(vaultId, {
        display_name: newName,
        email: newEmail,
        birthday: newBirthday || null,
        release_note: newReleaseNote || null
      });
      contacts = [...contacts.filter((x) => x.email !== c.email), c];
      newName = ''; newEmail = ''; newBirthday = ''; newReleaseNote = '';
    } catch (e) {
      contactError = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      savingContact = false;
    }
  }

  async function revokeContact(contactId: string) {
    try {
      await deleteVaultContact(vaultId, contactId);
      contacts = contacts.filter((c) => c.id !== contactId);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    }
  }

  let encLabel = $derived(
    vault?.tier === 'ZERO_KNOWLEDGE'
      ? 'Zero-Knowledge AES-256-GCM'
      : 'Encrypted AES-256-GCM (Honest Operator)'
  );
  let encTitle = $derived(
    vault?.tier === 'ZERO_KNOWLEDGE'
      ? 'Keys never leave your device. The operator cannot read letter content.'
      : 'Encrypted at rest. The operator holds the key but commits not to read without a valid release trigger.'
  );
</script>

<div class="page">
  {#if loading && !vault}
    <p class="dim">Loading vault…</p>
  {:else if !vault}
    <Banner kind="warn">Vault not found.</Banner>
    <p><a href="{base}/dashboard">Back to dashboard</a></p>
  {:else}
    <header class="page-head">
      <div>
        <div class="eyebrow">VAULT</div>
        <h1>{vault.name}</h1>
        <div class="meta">
          <StatusBadge state={vault.state} />
          &nbsp;·&nbsp;
          <span class="enc-badge" title={encTitle}>{encLabel}</span>
          &nbsp;·&nbsp; Cooling-off: {vault.cooling_off_seconds}s
          &nbsp;·&nbsp; Storage: {vault.storage_region_label}
          &nbsp;·&nbsp; Last attestation: {fmtRelative(vault.last_attestation_at)}
        </div>
      </div>
      <div class="actions">
        {#if vault.state === 'COOLING_OFF'}
          <Button variant="primary" onclick={doCancel} disabled={action === 'cancel'}>
            {action === 'cancel' ? 'Cancelling…' : 'Cancel release'}
          </Button>
        {:else if vault.state === 'ACTIVE'}
          <Button variant="secondary" onclick={doDrill} disabled={action === 'drill'}>
            {action === 'drill' ? 'Drilling…' : 'Run drill'}
          </Button>
          <Button variant="danger" onclick={doForceRelease} disabled={action === 'release'}>
            {action === 'release' ? 'Starting…' : 'Force release'}
          </Button>
        {/if}
      </div>
    </header>

    {#if notice}<Banner kind="ok">{notice}</Banner>{/if}
    {#if error}<Banner kind="warn">{error}</Banner>{/if}

    <Card eyebrow="LETTERS" title="Sealed Letters">
      {#snippet actions()}
        <Button href={`${base}/letters/new?vault=${vault!.id}`}>Compose new</Button>
      {/snippet}
      {#snippet children()}
        {#if letters.length === 0}
          <p>No Letters in this Vault yet.</p>
          <p class="dim">
            Begin with <a href={`${base}/letters/new?vault=${vault!.id}`}>your first Letter</a> —
            a message, credentials, or instructions for one named Recipient.
          </p>
        {:else}
          <table>
            <thead>
              <tr><th>Title</th><th>Recipient</th><th>Sealed</th></tr>
            </thead>
            <tbody>
              {#each letters as l (l.id)}
                <tr>
                  <td>{l.title}</td>
                  <td class="mono">{l.recipient_email}</td>
                  <td>{fmtDate(l.sealed_at)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      {/snippet}
    </Card>

    {#if multiRegion}
      <Card eyebrow="STORAGE" title="Attachment Storage Region">
        {#snippet children()}
          <p class="dim">
            Choose where this Vault's sealed files are stored. Letter text and
            all metadata always stay in our primary region — only attachment
            blobs move. A move re-encrypts each file in the new region and
            removes the old copy.
          </p>
          <div class="region-row">
            <select bind:value={targetRegion} disabled={movingRegion}>
              {#each STORAGE_REGIONS as r}
                <option value={r.code}>{r.label}</option>
              {/each}
            </select>
            <Button
              onclick={doMoveRegion}
              disabled={movingRegion || targetRegion === vault!.storage_region}
            >
              {movingRegion ? 'Moving…' : 'Move region'}
            </Button>
          </div>
          {#if targetRegion === vault!.storage_region}
            <p class="dim region-current">Currently stored in {vault!.storage_region_label}.</p>
          {/if}
        {/snippet}
      </Card>
    {/if}

    <Card eyebrow="CONTACTS" title="Vault Contacts">
      {#snippet children()}
        <p class="dim">
          Save named recipients to this Vault for quick selection when composing Letters.
        </p>
        {#if contacts.length > 0}
          <table style="margin-bottom: var(--sp-3)">
            <thead>
              <tr><th>Name</th><th>Email</th><th>Birthday</th><th>Release note</th><th></th></tr>
            </thead>
            <tbody>
              {#each contacts as c (c.id)}
                <tr>
                  <td>{c.display_name}</td>
                  <td class="mono">{c.email}</td>
                  <td>{c.birthday ?? '—'}</td>
                  <td class="dim">{c.release_note ?? '—'}</td>
                  <td>
                    <button type="button" class="revoke-btn" onclick={() => revokeContact(c.id)}>
                      Remove
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
        <form class="contact-form" onsubmit={saveContact}>
          <div class="contact-grid">
            <div class="field">
              <label>Name *</label>
              <input type="text" bind:value={newName} required placeholder="Alex Smith" />
            </div>
            <div class="field">
              <label>Email *</label>
              <input type="email" bind:value={newEmail} required placeholder="alex@example.com" />
            </div>
            <div class="field">
              <label>Birthday</label>
              <input type="date" bind:value={newBirthday} />
            </div>
            <div class="field">
              <label>Release note</label>
              <input type="text" bind:value={newReleaseNote} placeholder="Hold until she turns 18" />
            </div>
          </div>
          {#if contactError}<p class="err">{contactError}</p>{/if}
          <Button type="submit" disabled={savingContact}>
            {savingContact ? 'Saving…' : 'Save contact'}
          </Button>
        </form>
      {/snippet}
    </Card>
  {/if}
</div>

<style>
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--sp-4);
    gap: var(--sp-3);
  }
  .actions { display: flex; gap: var(--sp-2); }
  h1 { margin: 4px 0; font-size: var(--size-display-2); }
  .meta { font-size: var(--size-caption); color: var(--slate); margin-top: 4px; }
  .enc-badge {
    font-size: var(--size-micro);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--verdigris);
    border: 1px solid var(--verdigris);
    padding: 2px 6px;
    cursor: default;
  }
  .revoke-btn {
    background: none;
    border: none;
    color: var(--slate);
    font-size: var(--size-caption);
    text-decoration: underline;
    cursor: pointer;
    padding: 0;
  }
  .revoke-btn:hover { color: var(--burgundy); }
  .contact-form { margin-top: var(--sp-3); }
  .contact-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sp-2) var(--sp-3);
    margin-bottom: var(--sp-3);
  }
  @media (max-width: 600px) { .contact-grid { grid-template-columns: 1fr; } }
  .field { margin-bottom: 0; }
  .field label { display: block; font-size: var(--size-caption); color: var(--slate); margin-bottom: 4px; }
  .err { font-size: var(--size-caption); color: var(--burgundy); margin: 0 0 var(--sp-2); }
  .region-row { display: flex; gap: var(--sp-2); align-items: center; }
  .region-row select {
    padding: var(--sp-2);
    border: 1px solid var(--rule-ink);
    border-radius: var(--radius-sm, 4px);
    background: var(--paper, #fff);
    color: var(--ink);
    font: inherit;
  }
  .region-current { margin-top: var(--sp-2); }
</style>
