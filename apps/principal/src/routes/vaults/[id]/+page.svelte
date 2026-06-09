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
    getPasskeyAuthOptions,
    getZkEnvelopes,
    sealZkLetter,
    getZkLetterCiphertext,
    ApiError
  } from '$lib/api';
  import type { Letter, Vault, VaultContact } from '$lib/api';
  import {
    deriveVaultKeyViaPrf,
    unwrapVaultDek,
    encryptLetter,
    decryptLetter,
    letterAad,
    generateStrongPassphrase,
    deriveHeirKey,
    importAesGcmKey,
    xorBytes,
    toBase64Url,
    fromBase64Url,
    bytesToHex
  } from '$lib/webauthn';
  import { load as loadSession } from '$lib/session';
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

  // ZK vault unlock state
  let vaultDek = $state<CryptoKey | null>(null);
  let unlocking = $state(false);
  let unlockError = $state<string | null>(null);

  const isZk = $derived(vault?.tier === 'ZERO_KNOWLEDGE');

  // ZK encrypted compose + read-back state
  let zkComposeOpen = $state(false);
  let zkTitle = $state('');
  let zkRecipient = $state('');
  let zkBody = $state('');
  // Recipient access mode: none (author-only), manual passphrase, split
  // (auto zero-knowledge), or operator (auto convenient).
  let heirMode = $state<'none' | 'manual' | 'split' | 'operator'>('none');
  let zkPassphrase = $state('');
  let zkSealing = $state(false);
  let zkComposeError = $state<string | null>(null);
  let openBodies = $state<Record<string, string>>({}); // letterId → decrypted body
  let decrypting = $state<string | null>(null);
  let readError = $state<string | null>(null);

  async function sealZk(e: Event) {
    e.preventDefault();
    if (!vaultDek) { zkComposeError = 'Vault is locked.'; return; }
    if (!zkTitle.trim() || !zkBody.trim()) { zkComposeError = 'Title and message are required.'; return; }
    if (!zkRecipient.includes('@')) { zkComposeError = 'Recipient email looks invalid.'; return; }
    zkSealing = true;
    zkComposeError = null;
    try {
      // Encrypt the body in the browser; the operator only ever sees ciphertext.
      // AAD binds it to this vault so it can't be moved between vaults (M1).
      const { ciphertext, nonce } = await encryptLetter(vaultDek, zkBody, letterAad(vaultId));

      // Recipient access — seal the body a second time so the recipient can
      // open it after release. How they obtain the key depends on the mode.
      let heir:
        | {
            heir_ciphertext: string;
            heir_nonce: string;
            heir_mode: 'manual' | 'split' | 'operator';
            heir_salt?: string;
            heir_release_secret?: string;
            heir_recipient_share?: string;
          }
        | undefined;

      if (heirMode === 'manual') {
        const pass = zkPassphrase.trim();
        if (!pass) {
          zkComposeError = 'Enter a passphrase (tap Generate for a strong one), or pick another option.';
          return;
        }
        if (pass.length < 12) {
          zkComposeError =
            'That passphrase is too short to resist brute force — use 12+ characters, or tap Generate.';
          return;
        }
        const salt = crypto.getRandomValues(new Uint8Array(16));
        const heirKey = await deriveHeirKey(pass, salt);
        const sealed = await encryptLetter(heirKey, zkBody);
        heir = {
          heir_ciphertext: toBase64Url(sealed.ciphertext),
          heir_nonce: toBase64Url(sealed.nonce),
          heir_mode: 'manual',
          heir_salt: bytesToHex(salt)
        };
      } else if (heirMode === 'split') {
        // Auto, zero-knowledge: random key K = recipientShare XOR operatorShare.
        const k = crypto.getRandomValues(new Uint8Array(32));
        const sealed = await encryptLetter(await importAesGcmKey(k, ['encrypt']), zkBody);
        const recipientShare = crypto.getRandomValues(new Uint8Array(32));
        heir = {
          heir_ciphertext: toBase64Url(sealed.ciphertext),
          heir_nonce: toBase64Url(sealed.nonce),
          heir_mode: 'split',
          heir_release_secret: toBase64Url(xorBytes(k, recipientShare)),
          heir_recipient_share: toBase64Url(recipientShare)
        };
      } else if (heirMode === 'operator') {
        // Auto, convenient: operator holds the whole key, releases it on unseal.
        const k = crypto.getRandomValues(new Uint8Array(32));
        const sealed = await encryptLetter(await importAesGcmKey(k, ['encrypt']), zkBody);
        heir = {
          heir_ciphertext: toBase64Url(sealed.ciphertext),
          heir_nonce: toBase64Url(sealed.nonce),
          heir_mode: 'operator',
          heir_release_secret: toBase64Url(k)
        };
      }

      const letter = await sealZkLetter(vaultId, {
        title: zkTitle,
        recipient_email: zkRecipient,
        ciphertext: toBase64Url(ciphertext),
        nonce: toBase64Url(nonce),
        ...heir
      });
      letters = [...letters, letter];
      notice =
        heirMode === 'split'
          ? 'Letter sealed. We emailed the recipient their access code.'
          : heirMode === 'operator'
            ? 'Letter sealed. The recipient can open it when the vault unseals.'
            : heir
              ? 'Letter sealed. Share the passphrase with the recipient out-of-band.'
              : 'Letter sealed. Only you can read it back.';
      zkTitle = ''; zkRecipient = ''; zkBody = ''; zkPassphrase = ''; heirMode = 'none';
      zkComposeOpen = false;
    } catch (err) {
      zkComposeError = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
    } finally {
      zkSealing = false;
    }
  }

  async function viewZkLetter(letterId: string) {
    if (letterId in openBodies) {
      const { [letterId]: _drop, ...rest } = openBodies;
      openBodies = rest;
      return;
    }
    if (!vaultDek) { readError = 'Vault is locked.'; return; }
    decrypting = letterId;
    readError = null;
    try {
      const { ciphertext, nonce } = await getZkLetterCiphertext(vaultId, letterId);
      const ct = fromBase64Url(ciphertext);
      const n = fromBase64Url(nonce);
      let body: string;
      try {
        body = await decryptLetter(vaultDek, ct, n, letterAad(vaultId));
      } catch {
        // Legacy letters sealed before AAD binding (v1) decrypt without it.
        body = await decryptLetter(vaultDek, ct, n);
      }
      openBodies = { ...openBodies, [letterId]: body };
    } catch (err) {
      readError = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
    } finally {
      decrypting = null;
    }
  }

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

  async function unlockVault() {
    if (!vault) return;
    const session = loadSession();
    if (!session) { unlockError = 'Not signed in.'; return; }
    unlocking = true;
    unlockError = null;
    try {
      const { options } = await getPasskeyAuthOptions(session.email);
      const prfResult = await deriveVaultKeyViaPrf(options, vaultId);
      if (!prfResult) {
        unlockError = "Your passkey doesn't support client-side encryption (PRF). Try a newer device.";
        return;
      }
      const envelopes = await getZkEnvelopes(vaultId);
      if (envelopes.length === 0) {
        unlockError = 'No key envelopes found for this vault.';
        return;
      }
      let dek: CryptoKey | null = null;
      for (const env of envelopes) {
        try {
          dek = await unwrapVaultDek(prfResult.key, fromBase64Url(env.ciphertext), fromBase64Url(env.nonce));
          break;
        } catch { /* try next envelope */ }
      }
      if (!dek) {
        unlockError = 'Could not decrypt the vault key with this passkey. Try a different passkey.';
        return;
      }
      vaultDek = dek;
      notice = 'Vault unlocked for this session.';
    } catch (e) {
      unlockError = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      unlocking = false;
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
        {#if !isZk}
          <Button href={`${base}/letters/new?vault=${vault!.id}`}>Compose new</Button>
        {:else if vaultDek}
          <Button onclick={() => { zkComposeOpen = !zkComposeOpen; zkComposeError = null; }}>
            {zkComposeOpen ? 'Close' : 'Compose encrypted'}
          </Button>
        {/if}
      {/snippet}
      {#snippet children()}
        {#if isZk && !vaultDek}
          <div class="zk-lock">
            <p class="dim">
              This is a Private Vault. Letter content is encrypted in your browser.
              Unlock it with your passkey to view and compose letters.
            </p>
            {#if unlockError}
              <Banner kind="warn">{unlockError}</Banner>
            {/if}
            <Button onclick={unlockVault} disabled={unlocking}>
              {unlocking ? 'Unlocking…' : 'Unlock vault'}
            </Button>
          </div>
        {:else}
          {#if isZk && vaultDek && zkComposeOpen}
            <form class="zk-compose" onsubmit={sealZk}>
              <p class="dim small">
                The message is encrypted in your browser under this Vault's key.
                Paschal stores only ciphertext and can never read it.
              </p>
              <div class="zk-grid">
                <div class="field">
                  <label for="zk-title">Title *</label>
                  <input id="zk-title" type="text" bind:value={zkTitle} required placeholder="For you only" />
                </div>
                <div class="field">
                  <label for="zk-recipient">Recipient email *</label>
                  <input id="zk-recipient" type="email" bind:value={zkRecipient} required placeholder="alex@example.com" />
                </div>
              </div>
              <div class="field">
                <label for="zk-body">Message *</label>
                <textarea id="zk-body" bind:value={zkBody} rows="6" required placeholder="Write your encrypted letter…"></textarea>
              </div>
              <div class="field">
                <label for="zk-heir-mode">Recipient access</label>
                <select id="zk-heir-mode" bind:value={heirMode}>
                  <option value="none">Only I can read it back</option>
                  <option value="manual">I'll set a passphrase to share myself</option>
                  <option value="split">Auto — zero-knowledge (email the recipient a code now)</option>
                  <option value="operator">Auto — convenient (Paschal can read this letter)</option>
                </select>
                {#if heirMode === 'manual'}
                  <div class="zk-pass-row">
                    <input id="zk-pass" type="text" bind:value={zkPassphrase} autocomplete="off" placeholder="A strong passphrase to share with the recipient" />
                    <button type="button" class="text-link" onclick={() => (zkPassphrase = generateStrongPassphrase())}>Generate</button>
                  </div>
                {/if}
                <span class="zk-pass-help dim">
                  {#if heirMode === 'none'}
                    The recipient can't open this letter — only you can read it back.
                  {:else if heirMode === 'manual'}
                    You share the passphrase with the recipient yourself (e.g. written with your will).
                  {:else if heirMode === 'split'}
                    We email the recipient an access code now and hold the other half until the vault
                    unseals — we never hold both, so we can't read the letter. The recipient must keep
                    their code.
                  {:else}
                    We hold the key and give it to the recipient automatically when the vault unseals.
                    Nothing for them to keep — but for these letters, we technically could read them.
                  {/if}
                </span>
              </div>
              {#if zkComposeError}<Banner kind="warn">{zkComposeError}</Banner>{/if}
              <Button type="submit" disabled={zkSealing}>
                {zkSealing ? 'Encrypting & sealing…' : 'Encrypt & seal'}
              </Button>
            </form>
          {/if}

          {#if letters.length === 0}
            <p>No Letters in this Vault yet.</p>
            {#if isZk}
              <p class="dim">Use “Compose encrypted” to seal your first browser-encrypted Letter.</p>
            {:else}
              <p class="dim">
                Begin with <a href={`${base}/letters/new?vault=${vault!.id}`}>your first Letter</a> —
                a message, credentials, or instructions for one named Recipient.
              </p>
            {/if}
          {:else}
            <table>
              <thead>
                <tr><th>Title</th><th>Recipient</th><th>Sealed</th>{#if isZk}<th></th>{/if}</tr>
              </thead>
              <tbody>
                {#each letters as l (l.id)}
                  <tr>
                    <td>{l.title}</td>
                    <td class="mono">{l.recipient_email}</td>
                    <td>{fmtDate(l.sealed_at)}</td>
                    {#if isZk}
                      <td>
                        <button
                          type="button"
                          class="revoke-btn"
                          onclick={() => viewZkLetter(l.id)}
                          disabled={decrypting === l.id}
                        >
                          {decrypting === l.id ? 'Decrypting…' : (l.id in openBodies ? 'Hide' : 'View')}
                        </button>
                      </td>
                    {/if}
                  </tr>
                  {#if isZk && l.id in openBodies}
                    <tr class="zk-body-row">
                      <td colspan="4"><pre class="zk-body">{openBodies[l.id]}</pre></td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
            {#if readError}<Banner kind="warn">{readError}</Banner>{/if}
          {/if}
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
  .zk-lock { display: flex; flex-direction: column; gap: var(--sp-3); }

  /* ZK encrypted compose */
  .zk-compose {
    border: 1px solid var(--mist);
    border-left: 3px solid var(--verdigris);
    padding: var(--sp-3);
    margin-bottom: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .zk-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sp-2) var(--sp-3);
  }
  @media (max-width: 600px) { .zk-grid { grid-template-columns: 1fr; } }
  .zk-compose input,
  .zk-compose textarea,
  .zk-compose select {
    width: 100%;
    padding: var(--sp-2);
    border: 1px solid var(--rule-ink);
    border-radius: var(--radius-sm, 4px);
    background: var(--paper, #fff);
    color: var(--ink);
    font: inherit;
    box-sizing: border-box;
  }
  .zk-compose select + input { margin-top: var(--sp-2); }
  .zk-compose textarea { resize: vertical; }
  .zk-pass-row { display: flex; gap: var(--sp-2); align-items: center; margin-top: var(--sp-2); }
  .zk-pass-row input { flex: 1; }
  .zk-pass-help { display: block; margin-top: 4px; font-size: var(--size-caption); line-height: 1.4; }
  .zk-body-row td { padding: 0 0 var(--sp-2); }
  .zk-body {
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    background: var(--vellum);
    border-left: 2px solid var(--verdigris);
    font-family: inherit;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
