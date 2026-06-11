<script lang="ts">
  import { page } from '$app/state';
  import { base } from '$app/paths';
  import { confirmBuddy, ApiError } from '$lib/api';
  import Card from '$lib/components/Card.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let token = $state(page.url.searchParams.get('token') ?? '');
  let submitting = $state(false);
  let confirmed = $state(false);
  let error = $state<string | null>(null);

  async function confirm(e: Event) {
    e.preventDefault();
    submitting = true;
    error = null;
    try {
      await confirmBuddy(token);
      confirmed = true;
    } catch (err) {
      error = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="page narrow">
  {#if !token}
    <Card eyebrow="INVITATION" title="Invalid invitation link">
      <Banner kind="warn">
        This invitation link is missing its token. Please use the link from the
        email you received.
      </Banner>
      <p class="dim small"><a href="{base}/signin">Sign in instead</a></p>
    </Card>
  {:else if confirmed}
    <Card eyebrow="CONFIRMED" title="You're now a Paschal Buddy">
      <p>
        Thank you for accepting. You'll receive occasional check-in requests by
        email — just reply to confirm the person is well.
      </p>
      <p class="dim small">
        Want your own Vault? <a href="{base}/">Create an account</a>
      </p>
    </Card>
  {:else}
    <Card eyebrow="ACCEPT INVITATION" title="Become a Paschal Buddy">
      <p>
        You've been invited to act as a <strong>Buddy</strong> on someone's
        Paschal account.
      </p>
      <p class="dim">
        Buddies receive occasional check-in requests. If you don't respond,
        Paschal treats that as a signal that the person may be unreachable.
        You <em>never</em> see Letter contents.
      </p>

      {#if error}
        <Banner kind="warn">{error}</Banner>
      {/if}

      <form onsubmit={confirm}>
        <Button type="submit" disabled={submitting || !token}>
          {submitting ? 'Confirming…' : 'Accept Buddy role →'}
        </Button>
      </form>

      <p class="dim small" style="margin-top: 1rem;">
        Not sure what this is? <a href="{base}/">Learn about Paschal</a>
      </p>
    </Card>
  {/if}
</div>

<style>
  p { margin: 0 0 var(--sp-2); }
  .dim { color: var(--slate); }
</style>
