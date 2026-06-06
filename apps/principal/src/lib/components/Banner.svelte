<script lang="ts">
  import type { Snippet } from 'svelte';
  type Kind = 'info' | 'ok' | 'warn';
  type Props = { kind?: Kind; children: Snippet };
  let { kind = 'info', children }: Props = $props();
</script>

<div class="banner {kind}" role={kind === 'warn' ? 'alert' : 'status'}>
  {@render children()}
</div>

<style>
  /* Styles inherit from global .banner / .banner.ok / .banner.warn in app.css */
</style>
