<script lang="ts">
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { base } from '$app/paths';
  import { signout } from '$lib/api';
  import { clear } from '$lib/session';
  import Monogram from './Monogram.svelte';

  type Props = { email: string };
  let { email }: Props = $props();

  const links = [
    { href: `${base}/dashboard`, label: 'Dashboard' },
    { href: `${base}/buddies`, label: 'Buddies' },
    { href: `${base}/co-stewards`, label: 'Co-Stewards' },
    { href: `${base}/account`, label: 'Account' },
    { href: `${base}/transparency`, label: 'Transparency' },
    { href: `${base}/docs`, label: 'How it works' }
  ];

  async function signOut() {
    await signout().catch(() => {});
    clear();
    goto(`${base}/`);
  }
</script>

<nav>
  <a class="brand" href="{base}/dashboard">
    <Monogram size={32} />
    <span class="wordmark">Paschal</span>
  </a>
  <ul>
    {#each links as l (l.href)}
      <li class:active={page.url.pathname.startsWith(l.href)}>
        <a href={l.href}>{l.label}</a>
      </li>
    {/each}
  </ul>
  <div class="right">
    <span class="email" title={email}>{email}</span>
    <button type="button" onclick={signOut}>Sign out</button>
  </div>
</nav>

<style>
  nav {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: 12px var(--sp-3);
    border-top: 3px solid var(--ink);
    border-bottom: var(--rule-ink);
    background: var(--parchment);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    text-decoration: none;
  }
  .wordmark {
    font-family: var(--font-display);
    font-size: 24px;
    letter-spacing: -0.005em;
    color: var(--ink);
  }
  ul {
    display: flex;
    list-style: none;
    padding: 0;
    margin: 0;
    gap: var(--sp-3);
  }
  ul a {
    text-decoration: none;
    color: var(--slate);
    font-size: var(--size-body-2);
    padding: 4px 0;
    border-bottom: 2px solid transparent;
  }
  ul li.active a {
    color: var(--ink);
    border-bottom-color: var(--ink);
  }
  ul a:hover { color: var(--ink); }
  .right {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .email {
    font-size: var(--size-caption);
    color: var(--slate);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  button {
    background: none;
    border: none;
    color: var(--ink);
    font-size: var(--size-body-2);
    text-decoration: underline;
    text-underline-offset: 4px;
    cursor: pointer;
    padding: 4px 0;
  }
  button:hover { color: var(--burgundy); }
</style>
