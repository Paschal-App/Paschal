<script lang="ts">
  import '../app.css';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { load as loadSession } from '$lib/session';
  import NavBar from '$lib/components/NavBar.svelte';
  import Footer from '$lib/components/Footer.svelte';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();

  const PUBLIC = [
    `${base}/`,
    `${base}/signin`,
    `${base}/docs`,
    `${base}/co-steward/confirm`,
    `${base}/co-steward/sign-in`,
    `${base}/co-steward/dashboard`,
  ];
  let session = $state<ReturnType<typeof loadSession>>(null);

  $effect(() => {
    const s = loadSession();
    session = s;
    const path = page.url.pathname;
    const isPublic = PUBLIC.some((p) => path === p);
    if (!s && !isPublic) {
      goto(`${base}/signin`, { replaceState: true });
    }
  });
</script>

{#if session}
  <NavBar email={session.email} />
{/if}

<main>
  {@render children()}
</main>

<Footer />

<style>
  main {
    min-height: calc(100vh - 160px);
  }
</style>
