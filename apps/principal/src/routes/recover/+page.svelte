<script lang="ts">
  import { base } from '$app/paths';
  import { recoveryRedeem, ApiError } from '$lib/api';
  import {
    deriveRecoveryKey,
    unwrapPrk,
    unwrapVaultDek,
    decryptLetter,
    fromBase64Url,
    hexToBytes
  } from '$lib/webauthn';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let email = $state('');
  let code = $state('');
  let submitting = $state(false);
  let error = $state<string | null>(null);

  type RecoveredLetter = { id: string; title: string; recipient_email: string; body: string };
  type RecoveredVault = { vault_id: string; name: string; letters: RecoveredLetter[] };
  let recovered = $state<RecoveredVault[] | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    recovered = null;
    try {
      const bundle = await recoveryRedeem(email.trim(), code.trim());

      // Derive the recovery KEK from the code, unwrap the principal Recovery Key,
      // then unwrap each Vault DEK and decrypt each letter — all in the browser.
      const kek = await deriveRecoveryKey(code.trim(), hexToBytes(bundle.code_salt));
      const prk = await unwrapPrk(
        kek,
        fromBase64Url(bundle.prk_code_ct),
        fromBase64Url(bundle.prk_code_nonce)
      );

      const out: RecoveredVault[] = [];
      for (const v of bundle.vaults) {
        const dek = await unwrapVaultDek(
          prk,
          fromBase64Url(v.dek_prk_ct),
          fromBase64Url(v.dek_prk_nonce)
        );
        const letters: RecoveredLetter[] = [];
        for (const l of v.letters) {
          let body: string;
          try {
            body = await decryptLetter(dek, fromBase64Url(l.ciphertext), fromBase64Url(l.nonce));
          } catch {
            body = '⚠ Could not decrypt this letter.';
          }
          letters.push({ id: l.id, title: l.title, recipient_email: l.recipient_email, body });
        }
        out.push({ vault_id: v.vault_id, name: v.name, letters });
      }
      recovered = out;
    } catch (err) {
      error =
        err instanceof ApiError
          ? err.problem.detail || err.problem.title
          : 'Could not recover with that code. Check the email and recovery code.';
    } finally {
      submitting = false;
    }
  }
</script>

<div class="page narrow">
  <Card eyebrow="RECOVER" title="Recover a Private Vault">
    {#if !recovered}
      <p class="dim">
        Lost your passkey? Enter your account email and the recovery code you
        saved when you created your account. Your letters are decrypted in your
        browser — Paschal never sees the recovery code or the contents.
      </p>
      <form onsubmit={submit}>
        <Field bind:value={email} label="Email" name="email" type="email" autocomplete="email" required />
        <Field bind:value={code} label="Recovery code" name="code" required />
        {#if error}<Banner kind="warn">{error}</Banner>{/if}
        <div class="row">
          <Button type="submit" disabled={submitting || !email || !code}>
            {submitting ? 'Recovering…' : 'Recover →'}
          </Button>
          <a class="text-link" href="{base}/signin">Back to sign in</a>
        </div>
      </form>
    {:else}
      <Banner kind="ok">
        Recovered {recovered.length} Private Vault{recovered.length === 1 ? '' : 's'}.
      </Banner>
      {#if recovered.length === 0}
        <p class="dim">No recoverable Private Vaults found for this account.</p>
      {/if}
      {#each recovered as v (v.vault_id)}
        <section class="rv">
          <h3>{v.name}</h3>
          {#if v.letters.length === 0}
            <p class="dim">No letters in this Vault.</p>
          {:else}
            {#each v.letters as l (l.id)}
              <article class="rl">
                <div class="rl-head">
                  <strong>{l.title}</strong> <span class="dim">→ {l.recipient_email}</span>
                </div>
                <pre class="rl-body">{l.body}</pre>
              </article>
            {/each}
          {/if}
        </section>
      {/each}
      <p class="dim small">
        Save anything you need now. To restore full access, sign in and register a new passkey.
      </p>
    {/if}
  </Card>
</div>

<style>
  .row { display: flex; gap: var(--sp-3); align-items: center; margin-top: var(--sp-2); }
  .text-link { color: var(--slate); text-decoration: underline; text-underline-offset: 4px; }
  .text-link:hover { color: var(--ink); }
  .small { font-size: var(--size-caption); }
  .rv { margin-top: var(--sp-3); }
  .rv h3 { margin: 0 0 var(--sp-2); font-size: 20px; }
  .rl {
    border: 1px solid var(--mist);
    border-left: 3px solid var(--verdigris);
    padding: var(--sp-2) var(--sp-3);
    margin-bottom: var(--sp-2);
  }
  .rl-head { margin-bottom: var(--sp-1); }
  .rl-body {
    margin: 0;
    font-family: inherit;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
