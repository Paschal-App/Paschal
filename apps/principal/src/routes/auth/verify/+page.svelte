<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { page } from '$app/state';
  import { verifyMagicLink, ApiError } from '$lib/api';
  import { save } from '$lib/session';
  import Card from '$lib/components/Card.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let status = $state<'verifying' | 'error'>('verifying');
  let error = $state<string | null>(null);

  $effect(() => {
    const token = page.url.searchParams.get('token');
    if (!token) {
      status = 'error';
      error = 'This sign-in link is missing its token. Request a new one.';
      return;
    }
    verifyMagicLink(token)
      .then((r) => {
        save({ token: r.session_token, email: r.email, principalId: r.principal_id });
        goto(`${base}/dashboard`);
      })
      .catch((err) => {
        status = 'error';
        error =
          err instanceof ApiError && err.problem.status === 401
            ? 'This sign-in link has expired or was already used. Request a new one.'
            : err instanceof ApiError
              ? err.problem.detail || err.problem.title
              : String(err);
      });
  });
</script>

<div class="page">
  {#if status === 'verifying'}
    <Card eyebrow="SIGNING IN" title="Verifying your sign-in link…">
      <p class="dim">One moment.</p>
    </Card>
  {:else}
    <Card eyebrow="SIGN-IN LINK" title="We couldn't sign you in">
      {#if error}<Banner kind="warn">{error}</Banner>{/if}
      <p class="dim small">
        <a href="{base}/signin">Request a new sign-in link</a>
      </p>
    </Card>
  {/if}
</div>

<style>
  .page {
    max-width: 560px;
    margin: 0 auto;
    width: 100%;
  }
  .small {
    font-size: var(--size-caption);
  }
  a {
    color: var(--ink);
    text-underline-offset: 4px;
  }
</style>
