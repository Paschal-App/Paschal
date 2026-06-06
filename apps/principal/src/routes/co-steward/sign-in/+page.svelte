<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { signInCoSteward, ApiError } from '$lib/api';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let email = $state('');
  let passphrase = $state('');
  let submitting = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    try {
      const resp = await signInCoSteward(email, passphrase);
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
  <Card eyebrow="CO-STEWARD" title="Sign in to view a dashboard">
    <p class="dim">
      If someone invited you to be their Co-Steward on Paschal, sign in with
      the email the invitation came to and the passphrase you chose at
      confirmation.
    </p>
    <form onsubmit={submit}>
      <Field
        bind:value={email}
        label="Email"
        name="email"
        type="email"
        autocomplete="email"
        required
      />
      <Field
        bind:value={passphrase}
        label="Passphrase"
        name="passphrase"
        type="password"
        required
      />
      {#if error}<Banner kind="warn">{error}</Banner>{/if}
      <Button type="submit" disabled={submitting || !email || !passphrase}>
        {submitting ? 'Signing in…' : 'Sign in'}
      </Button>
    </form>
    <p class="dim small" style="margin-top: var(--sp-2)">
      Not a Co-Steward? <a href="{base}/signin">Sign up as a Principal</a>.
    </p>
  </Card>
</div>

<style>
  p { margin: 0 0 var(--sp-2); }
  .dim { color: var(--slate); }
  .small { font-size: var(--size-caption); }
</style>
