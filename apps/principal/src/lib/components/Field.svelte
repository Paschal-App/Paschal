<script lang="ts">
  type Props = {
    label: string;
    value: string;
    name?: string;
    type?: 'text' | 'email' | 'password' | 'datetime-local' | 'number';
    placeholder?: string;
    required?: boolean;
    help?: string;
    error?: string;
    autocomplete?: string;
    min?: string;
    list?: string;
    onchange?: (v: string) => void;
  };

  let {
    label,
    value = $bindable(''),
    name,
    type = 'text',
    placeholder,
    required = false,
    help,
    error,
    autocomplete,
    min,
    list,
    onchange
  }: Props = $props();

  function handle(e: Event) {
    const t = e.target as HTMLInputElement;
    value = t.value;
    onchange?.(t.value);
  }
</script>

<div class="field">
  <label for={name}>
    {label}
    {#if required}<span aria-hidden="true">*</span>{/if}
  </label>
  <input
    id={name}
    {name}
    {type}
    {placeholder}
    {required}
    {autocomplete}
    {min}
    {list}
    {value}
    oninput={handle}
  />
  {#if help && !error}<p class="help">{help}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
</div>

<style>
  .field { margin-bottom: var(--sp-3); }
  .help, .error {
    font-size: var(--size-caption);
    margin: 6px 0 0;
  }
  .help { color: var(--slate); }
  .error { color: var(--burgundy); }
  label span { color: var(--burgundy); margin-left: 2px; }
</style>
