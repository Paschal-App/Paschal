<script lang="ts">
  type Props = {
    label: string;
    value: string;
    name?: string;
    rows?: number;
    placeholder?: string;
    required?: boolean;
    help?: string;
    error?: string;
  };

  let {
    label,
    value = $bindable(''),
    name,
    rows = 8,
    placeholder,
    required = false,
    help,
    error
  }: Props = $props();
</script>

<div class="field">
  <label for={name}>
    {label}
    {#if required}<span aria-hidden="true">*</span>{/if}
  </label>
  <textarea
    id={name}
    {name}
    {rows}
    {placeholder}
    {required}
    bind:value
  ></textarea>
  {#if help && !error}<p class="help">{help}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
</div>

<style>
  .field { margin-bottom: var(--sp-3); }
  .help, .error { font-size: var(--size-caption); margin: 6px 0 0; }
  .help { color: var(--slate); }
  .error { color: var(--burgundy); }
  label span { color: var(--burgundy); margin-left: 2px; }
</style>
