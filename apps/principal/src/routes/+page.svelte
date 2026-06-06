<script lang="ts">
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { load as loadSession } from '$lib/session';
  import Monogram from '$lib/components/Monogram.svelte';
  import Button from '$lib/components/Button.svelte';

  $effect(() => {
    if (loadSession()) goto(`${base}/dashboard`, { replaceState: true });
  });
</script>

<div class="page narrow">
  <div class="hero">
    <Monogram size={72} />
    <h1>Paschal</h1>
    <p class="lede">
      A digital letter of last resort.
    </p>
  </div>

  <div class="copy">
    <p>
      Paschal holds what matters in a sealed Vault, and releases it to the
      people you named when you are no longer able to reach it. Two postures
      are offered, and both are built so that the choice between them is
      yours, not ours.
    </p>
    <p>
      The system watches for signs of your absence — missed heartbeats,
      bank dormancy, family attestation — and waits, with a long cooling-off
      window you can always interrupt, before delivering anything.
    </p>
  </div>

  <div class="cta">
    <Button href="{base}/signin">Begin</Button>
    <a class="text-link" href="{base}/docs">How it works</a>
  </div>

  <hr />

  <p class="dim small">
    Three minutes of setup. Twenty-five years of patience. One delivery, if it
    ever has to happen.
  </p>
</div>

<style>
  .hero { text-align: center; margin-top: var(--sp-5); }
  .hero h1 {
    font-size: 80px;
    margin: var(--sp-2) 0 var(--sp-1);
  }
  .lede {
    font-family: var(--font-display);
    font-size: var(--size-display-3);
    color: var(--ink);
    margin: 0 0 var(--sp-5);
    font-weight: 400;
  }
  .copy {
    font-size: var(--size-body);
    line-height: 1.7;
    max-width: 560px;
    margin: 0 auto var(--sp-4);
  }
  .copy p { margin: 0 0 var(--sp-3); }
  .cta {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    margin-bottom: var(--sp-4);
  }
  .text-link {
    text-decoration: underline;
    text-underline-offset: 4px;
    color: var(--slate);
  }
  .text-link:hover { color: var(--ink); }
  .small { font-size: var(--size-caption); text-align: center; }
</style>
