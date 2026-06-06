<script lang="ts">
  import type { VaultState } from '$lib/types';
  type Props = { state: VaultState };
  let { state }: Props = $props();
  const cls = $derived(state.toLowerCase().replace('_', '-'));
  const label = $derived(
    {
      ACTIVE: 'Active',
      SUSPICIOUS: 'Suspicious',
      ALERT: 'Alert',
      COOLING_OFF: 'Cooling-off',
      RELEASING: 'Releasing',
      RELEASED: 'Released',
      ARCHIVED: 'Archived'
    }[state] ?? state
  );
</script>

<span class="badge"><span class="pip {cls}"></span>{label}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    font-size: var(--size-body-2);
    font-weight: 500;
  }
</style>
