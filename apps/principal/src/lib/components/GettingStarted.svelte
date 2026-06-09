<script lang="ts">
  import { base } from '$app/paths';
  import type { Vault, Buddy } from '$lib/types';
  import type { BankSignalStatus } from '$lib/api';

  type Props = {
    vaults: Vault[];
    buddies: Buddy[];
    bankSignal: BankSignalStatus | null;
    loading: boolean;
  };

  let { vaults, buddies, bankSignal, loading }: Props = $props();

  const STORAGE_KEY = 'paschal_onboarding_v1';

  let dismissed = $state(false);

  $effect(() => {
    dismissed = localStorage.getItem(STORAGE_KEY) === '1';
  });

  function dismiss() {
    localStorage.setItem(STORAGE_KEY, '1');
    dismissed = true;
  }

  function scrollToSignals() {
    document.getElementById('signals')?.scrollIntoView({ behavior: 'smooth' });
  }

  type Step = {
    key: string;
    label: string;
    description: string;
    done: boolean;
    href?: string;
    cta?: string;
    action?: () => void;
  };

  const steps: Step[] = $derived([
    {
      key: 'account',
      label: 'Created your account',
      description: '',
      done: true,
    },
    {
      key: 'vault',
      label: 'Create your first Vault',
      description:
        "A Vault holds letters for one purpose — your family, your business partner, a trusted friend. Nothing inside it goes anywhere until the signal system confirms you're unreachable.",
      done: vaults.length > 0,
      href: `${base}/vaults/new`,
      cta: 'Create a Vault',
    },
    {
      key: 'buddy',
      label: 'Invite a Buddy',
      description:
        "Buddies are trusted people Paschal can check in with. If they stop hearing from you, it contributes to the decision to release your letters. Aim for at least two.",
      done: buddies.length > 0,
      href: `${base}/buddies`,
      cta: 'Invite a Buddy',
    },
    {
      key: 'signal',
      label: 'Set up a proof-of-life signal',
      description:
        "Paschal needs a way to know you're still around. Connect your bank activity as a passive background signal, or press Heartbeat above to check in manually.",
      done: bankSignal !== null,
      action: scrollToSignals,
      cta: 'Go to Signals ↓',
    },
  ]);

  const allDone = $derived(steps.every(s => s.done));
  const show = $derived(!loading && !dismissed && !allDone);
  const currentKey = $derived(steps.find(s => !s.done)?.key ?? null);

  // Render completed steps first so the list always reads ✓ … ✓ → ○ … ○ — a
  // step completed out of order never appears below one that's still pending.
  // (Array.sort is stable, so declaration order is preserved within each group.)
  const orderedSteps = $derived([...steps].sort((a, b) => Number(b.done) - Number(a.done)));
</script>

{#if show}
  <section class="getting-started" aria-label="Getting started">
    <div class="gs-head">
      <div>
        <div class="eyebrow">GETTING STARTED</div>
        <h2>Set up your account</h2>
      </div>
      <button class="skip-btn" type="button" onclick={dismiss}>Skip</button>
    </div>
    <ol class="steps" role="list">
      {#each orderedSteps as step}
        {@const isCurrent = step.key === currentKey}
        <li
          class="step"
          class:done={step.done}
          class:current={isCurrent}
          class:future={!step.done && !isCurrent}
        >
          <span class="step-icon" aria-hidden="true">
            {#if step.done}✓{:else if isCurrent}→{:else}○{/if}
          </span>
          <div class="step-body">
            <div class="step-label">{step.label}</div>
            {#if isCurrent && step.description}
              <p class="step-desc">{step.description}</p>
              {#if step.href}
                <a class="step-cta" href={step.href}>{step.cta}</a>
              {:else if step.action}
                <button class="step-cta" type="button" onclick={step.action}>{step.cta}</button>
              {/if}
            {/if}
          </div>
        </li>
      {/each}
    </ol>
  </section>
{/if}

<style>
  .getting-started {
    background: var(--vellum);
    border: 1px solid var(--mist);
    border-left: 3px solid var(--burgundy);
    padding: var(--sp-4) var(--sp-5);
    margin-bottom: var(--sp-3);
    box-shadow: 0 1px 4px rgba(14, 27, 51, 0.07);
  }

  .gs-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-3);
    border-bottom: 1px solid var(--mist);
    padding-bottom: var(--sp-3);
    margin-bottom: var(--sp-3);
  }

  h2 {
    margin: 4px 0 0 0;
    font-size: var(--size-display-3);
  }

  .skip-btn {
    background: none;
    border: none;
    color: var(--slate);
    font-family: inherit;
    font-size: var(--size-body-2);
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
    text-underline-offset: 4px;
    flex-shrink: 0;
    align-self: flex-end;
    margin-bottom: 2px;
  }
  .skip-btn:hover { color: var(--ink); }

  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .step {
    display: flex;
    gap: var(--sp-3);
    padding: var(--sp-2) 0;
    border-bottom: 1px solid var(--mist);
  }
  .step:last-child { border-bottom: none; }

  .step-icon {
    font-size: 1rem;
    width: 20px;
    flex-shrink: 0;
    padding-top: 3px;
    line-height: 1;
  }

  .step-body {
    flex: 1;
    min-width: 0;
  }

  .step-label {
    font-size: var(--size-body-2);
    font-weight: 600;
    line-height: 1.4;
  }

  .done .step-icon { color: var(--verdigris); }
  .done .step-label { color: var(--slate); font-weight: 400; }

  .current .step-icon { color: var(--burgundy); font-weight: 700; }
  .current .step-label { color: var(--ink); }

  .future .step-icon { color: var(--slate); }
  .future .step-label { color: var(--slate); font-weight: 400; }

  .step-desc {
    margin: var(--sp-1) 0 var(--sp-2);
    font-size: var(--size-body-2);
    color: var(--slate);
    line-height: 1.55;
    max-width: 560px;
  }

  .step-cta {
    display: inline-block;
    background: var(--ink);
    color: var(--parchment);
    font-family: inherit;
    font-size: var(--size-body-2);
    font-weight: 600;
    padding: 8px 18px;
    border: 1px solid var(--ink);
    text-decoration: none;
    cursor: pointer;
    transition: background-color 120ms ease, border-color 120ms ease;
  }
  .step-cta:hover {
    background: var(--burgundy);
    border-color: var(--burgundy);
    color: var(--parchment);
  }
</style>
