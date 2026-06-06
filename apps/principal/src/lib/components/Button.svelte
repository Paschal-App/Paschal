<script lang="ts">
  import type { Snippet } from 'svelte';

  type Variant = 'primary' | 'secondary' | 'danger' | 'text';
  type Props = {
    variant?: Variant;
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    onclick?: (e: MouseEvent) => void;
    href?: string;
    children: Snippet;
  };

  let {
    variant = 'primary',
    type = 'button',
    disabled = false,
    onclick,
    href,
    children
  }: Props = $props();
</script>

{#if href}
  <a class="btn {variant}" {href}>{@render children()}</a>
{:else}
  <button class="btn {variant}" {type} {disabled} {onclick}>{@render children()}</button>
{/if}

<style>
  .btn {
    display: inline-block;
    font-family: var(--font-text);
    font-weight: 600;
    font-size: var(--size-body-2);
    line-height: 1.2;
    padding: 10px 20px;
    border: 1px solid var(--ink);
    background: var(--ink);
    color: var(--parchment);
    text-decoration: none;
    cursor: pointer;
    border-radius: 0;
    transition: background-color 120ms ease, color 120ms ease;
  }
  .btn:hover:not(:disabled) {
    background: var(--burgundy);
    border-color: var(--burgundy);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .secondary {
    background: transparent;
    color: var(--ink);
  }
  .secondary:hover:not(:disabled) {
    background: var(--ink);
    color: var(--parchment);
  }
  .danger {
    background: var(--burgundy);
    border-color: var(--burgundy);
    color: var(--parchment);
  }
  .danger:hover:not(:disabled) {
    background: var(--burgundy-deep);
    border-color: var(--burgundy-deep);
  }
  .text {
    background: transparent;
    border: none;
    color: var(--ink);
    text-decoration: underline;
    text-underline-offset: 4px;
    padding: 4px 0;
  }
  .text:hover:not(:disabled) {
    color: var(--burgundy);
  }
</style>
