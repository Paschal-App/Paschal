<script lang="ts">
  import type { Snippet } from 'svelte';
  type Props = { title?: string; eyebrow?: string; id?: string; children: Snippet; actions?: Snippet };
  let { title, eyebrow, id, children, actions }: Props = $props();
</script>

<section class="card" {id}>
  {#if eyebrow || title || actions}
    <header>
      <div>
        {#if eyebrow}<div class="eyebrow">{eyebrow}</div>{/if}
        {#if title}<h2>{title}</h2>{/if}
      </div>
      {#if actions}<div class="actions">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class="body">{@render children()}</div>
</section>

<style>
  .card {
    background: var(--vellum);
    border: 1px solid var(--mist);
    padding: var(--sp-4) var(--sp-5);
    margin-bottom: var(--sp-3);
    box-shadow: 0 1px 4px rgba(14, 27, 51, 0.07);
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-3);
    margin-bottom: var(--sp-3);
    border-bottom: 1px solid var(--mist);
    padding-bottom: var(--sp-3);
  }
  .actions { display: flex; gap: var(--sp-2); align-items: center; }
  h2 { margin: 4px 0 0 0; font-size: var(--size-display-3); }
</style>
