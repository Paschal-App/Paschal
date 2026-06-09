<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import {
    createVault,
    getSubscription,
    listPlans,
    getPasskeyAuthOptions,
    storeZkEnvelope,
    getRecoveryKey,
    storeVaultPrkEnvelope,
    STORAGE_REGIONS,
    ApiError
  } from '$lib/api';
  import {
    runPrfCeremony,
    deriveVaultKeyFromPrf,
    generateVaultDek,
    wrapVaultDek,
    derivePrkWrapKeyFromPrf,
    unwrapPrk,
    wrapAesKey,
    toBase64Url,
    fromBase64Url,
  } from '$lib/webauthn';
  import { load as loadSession } from '$lib/session';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let name = $state('');
  let tier = $state<'HONEST_OPERATOR' | 'ZERO_KNOWLEDGE'>('HONEST_OPERATOR');
  let coolingOffSeconds = $state('1209600'); // 14 days
  let storageRegion = $state(STORAGE_REGIONS[0]?.code ?? '');
  let multiRegion = $state(false);
  let submitting = $state(false);
  let error = $state<string | null>(null);

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
      if (tier === 'ZERO_KNOWLEDGE') {
        await submitZkVault();
      } else {
        const v = await createVault({
          name,
          cooling_off_seconds: parseInt(coolingOffSeconds, 10),
          ...(multiRegion ? { storage_region: storageRegion } : {})
        });
        goto(`${base}/vaults/${v.id}`);
      }
    } catch (err) {
      error = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
    } finally {
      submitting = false;
    }
  }

  async function submitZkVault() {
    const session = loadSession();
    if (!session) throw new Error('Not signed in');

    // One passkey ceremony — the PRF output expands into both the vault key
    // (sealed under this passkey) and the recovery-key wrap key.
    const { options } = await getPasskeyAuthOptions(session.email);
    const prf = await runPrfCeremony(options);
    if (!prf) {
      throw new Error(
        "Your passkey doesn't support client-side encryption. " +
        "Use Standard tier or a newer device (e.g. Face ID, Windows Hello, or a recent hardware key)."
      );
    }

    const v = await createVault({
      name,
      tier: 'ZERO_KNOWLEDGE',
      cooling_off_seconds: parseInt(coolingOffSeconds, 10),
      ...(multiRegion ? { storage_region: storageRegion } : {})
    });

    // Seal the DEK under the passkey-derived vault key (primary unlock path).
    const dek = await generateVaultDek();
    const vaultKey = await deriveVaultKeyFromPrf(prf.prfOutput, v.id);
    const wrapped = await wrapVaultDek(vaultKey, dek);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const passkeyId = (prf.credential as any).id ?? '';
    await storeZkEnvelope(v.id, {
      passkey_id: passkeyId,
      ciphertext: toBase64Url(wrapped.ciphertext),
      nonce: toBase64Url(wrapped.nonce)
    });

    // Arm recovery: also seal the DEK under the principal Recovery Key, so a
    // lost passkey can be recovered with the recovery code. Best-effort.
    try {
      const rk = await getRecoveryKey();
      if (rk.configured && rk.prk_prf_ct && rk.prk_prf_nonce) {
        const prkWrapKey = await derivePrkWrapKeyFromPrf(prf.prfOutput);
        const prk = await unwrapPrk(
          prkWrapKey,
          fromBase64Url(rk.prk_prf_ct),
          fromBase64Url(rk.prk_prf_nonce)
        );
        const dekUnderPrk = await wrapAesKey(prk, dek);
        await storeVaultPrkEnvelope(v.id, {
          ciphertext: toBase64Url(dekUnderPrk.ciphertext),
          nonce: toBase64Url(dekUnderPrk.nonce)
        });
      }
    } catch (e) {
      console.warn('Vault recovery envelope setup failed', e);
    }

    goto(`${base}/vaults/${v.id}`);
  }
</script>

<div class="page narrow">
  <Card eyebrow="NEW VAULT" title="Create a Vault">
    <form onsubmit={submit}>
      <p class="dim">
        A Vault is a container for Letters. You can have several, each with
        its own cooling-off and signal policy.
      </p>

      <!-- Tier selector -->
      <fieldset class="tier-fieldset">
        <legend class="sr-only">Vault encryption tier</legend>

        <label class="tier-option" class:selected={tier === 'HONEST_OPERATOR'}>
          <input
            type="radio"
            name="tier"
            value="HONEST_OPERATOR"
            bind:group={tier}
          />
          <div class="tier-text">
            <strong>Standard</strong>
            <span class="dim">
              Letters are encrypted at rest. Paschal manages keys.
              You can always read your letters back.
            </span>
          </div>
        </label>

        <label class="tier-option" class:selected={tier === 'ZERO_KNOWLEDGE'}>
          <input
            type="radio"
            name="tier"
            value="ZERO_KNOWLEDGE"
            bind:group={tier}
          />
          <div class="tier-text">
            <strong>Private</strong>
            <span class="dim">
              Letters are encrypted in your browser. Paschal never
              has access to the key. Requires a passkey with PRF support
              (Face ID, Windows Hello, or YubiKey 5+).
            </span>
          </div>
        </label>
      </fieldset>

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

      {#if tier === 'ZERO_KNOWLEDGE'}
        <Banner kind="warn">
          You will be prompted for your passkey <strong>twice</strong> during
          Private Vault creation — once to verify PRF support, and once to
          derive the vault key. Keep your passkey handy.
        </Banner>
      {/if}

      {#if error}
        <Banner kind="warn">{error}</Banner>
      {/if}

      <div class="row">
        <Button type="submit" disabled={submitting || !name.trim()}>
          {submitting ? 'Creating…' : 'Create Vault'}
        </Button>
        <a class="text-link" href="{base}/dashboard">Cancel</a>
      </div>
    </form>
  </Card>
</div>

<style>
  /* Tier selector */
  .tier-fieldset {
    border: none;
    padding: 0;
    margin: 0 0 var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .tier-option {
    display: flex;
    gap: var(--sp-2);
    align-items: flex-start;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--mist);
    cursor: pointer;
    transition: border-color 120ms ease;
  }
  .tier-option.selected { border-color: var(--ink); }
  .tier-option input[type="radio"] { margin-top: 3px; flex-shrink: 0; }
  .tier-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--size-body-2);
  }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0,0,0,0); }

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
