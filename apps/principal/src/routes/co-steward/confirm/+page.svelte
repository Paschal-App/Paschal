<script lang="ts">
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { confirmCoSteward, ApiError } from '$lib/api';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let token = $state(page.url.searchParams.get('token') ?? '');
  let passphrase = $state('');
  let confirmPp = $state('');
  let submitting = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    if (passphrase !== confirmPp) {
      error = 'Passphrases do not match.';
      return;
    }
    if (passphrase.length < 10) {
      error = 'Passphrase must be at least 10 characters.';
      return;
    }
    submitting = true;
    error = null;
    try {
      const resp = await confirmCoSteward(token, passphrase);
      // Stash the session locally and head to the dashboard.
      localStorage.setItem(
        'paschal_co_steward_session',
        JSON.stringify({
          token: resp.session_token,
          co_steward_id: resp.co_steward_id
        })
      );
      goto(`${base}/co-steward/dashboard`);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="page narrow">
  <Card eyebrow="ACCEPT INVITATION" title="Become a Co-Steward">
    <p>
      You've been invited to act as a <strong>Co-Steward</strong> on someone's
      Paschal account.
    </p>
    <p class="dim">
      Co-Stewards can see the state of the principal's Vaults — last heartbeat,
      signal strength, scheduled releases, plan tier. They <em>never</em> see
      Letter contents and <em>cannot change anything</em>.
    </p>
    <p class="dim">
      Choose a passphrase below. You'll use it together with the email this
      invitation came to whenever you sign in.
    </p>

    <form onsubmit={submit}>
      <Field
        bind:value={token}
        label="Invitation token"
        name="token"
        required
        help="Should be pre-filled from the URL."
      />
      <Field
        bind:value={passphrase}
        label="Choose a passphrase"
        name="passphrase"
        type="password"
        required
        help="At least 10 characters. The principal does not see this."
      />
      <Field
        bind:value={confirmPp}
        label="Confirm passphrase"
        name="passphrase_confirm"
        type="password"
        required
      />

      {#if error}<Banner kind="warn">{error}</Banner>{/if}

      <Button type="submit" disabled={submitting || !token || !passphrase}>
        {submitting ? 'Confirming…' : 'Accept and sign in'}
      </Button>
    </form>
  </Card>
</div>

<style>
  p { margin: 0 0 var(--sp-2); }
  .dim { color: var(--slate); }
</style>
