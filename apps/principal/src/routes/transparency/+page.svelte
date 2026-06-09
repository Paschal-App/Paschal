<script lang="ts">
  import { getWarrantCanary, getTransparencyLog, ApiError, type TransparencyEntry } from '$lib/api';
  import { fmtDate } from '$lib/format';
  import Card from '$lib/components/Card.svelte';
  import Banner from '$lib/components/Banner.svelte';

  let canary = $state<{ statement: string; issued_at: string; next_update_by: string } | null>(null);
  let log = $state<TransparencyEntry[]>([]);
  let canaryError = $state<string | null>(null);
  let logError = $state<string | null>(null);

  $effect(() => {
    getWarrantCanary()
      .then((c) => (canary = c))
      .catch((e) => {
        canaryError = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
      });

    getTransparencyLog()
      .then((entries) => (log = entries))
      .catch((e) => {
        logError = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
      });
  });
</script>

<div class="page">
  <header class="head">
    <div class="eyebrow">TRANSPARENCY</div>
    <h1>Transparency &amp; Trust</h1>
    <p class="lede">
      Paschal keeps a tamper-evident log of significant events and publishes a
      warrant canary, so you can verify the operator of this instance has not been
      compelled to act against your interests.
    </p>
  </header>

  <!-- Warrant Canary -->
  <Card eyebrow="WARRANT CANARY" title="What the operator has not been asked to do">
    {#if canaryError}
      <Banner kind="warn">{canaryError}</Banner>
    {:else if !canary}
      <p class="dim">Loading…</p>
    {:else}
      <blockquote class="canary-statement">
        {canary.statement}
      </blockquote>
      <div class="canary-meta">
        <span>Issued: <strong>{fmtDate(canary.issued_at)}</strong></span>
        <span>Next update by: <strong>{fmtDate(canary.next_update_by)}</strong></span>
      </div>
      <p class="dim small">
        If this statement disappears or is not updated by the date above,
        treat it as a signal that the operator may have received a gag order.
        The absence of the canary is the warning.
      </p>
    {/if}
  </Card>

  <!-- Transparency Log -->
  <Card eyebrow="TRANSPARENCY LOG" title="Significant events">
    <p class="dim">
      A tamper-evident record of significant operations — account creation,
      vault releases, and key management events. Each entry stores a payload
      hash so the log can be checked for tampering.
    </p>
    {#if logError}
      <Banner kind="warn">{logError}</Banner>
    {:else if log.length === 0}
      <p class="dim" style="margin-top: var(--sp-2)">No entries yet — or loading…</p>
    {:else}
      <div class="log-table-wrap">
        <table class="log-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Event</th>
              <th>ID</th>
            </tr>
          </thead>
          <tbody>
            {#each log as entry (entry.id)}
              <tr>
                <td class="mono small nowrap">{fmtDate(entry.ts)}</td>
                <td>
                  <span class="kind-pill">{entry.kind}</span>
                </td>
                <td class="mono small dim">{entry.id.slice(0, 8)}…</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="dim small" style="margin-top: var(--sp-2)">
        Showing the {log.length} most recent entries.
      </p>
    {/if}
  </Card>

  <!-- Continuity for self-hosted operators -->
  <Card eyebrow="CONTINUITY" title="If this instance goes offline">
    <p class="dim">
      This is a self-hosted Paschal instance — its continuity depends on the
      operator's backups, not on any third-party escrow:
    </p>
    <ol class="protocol-list">
      <li>
        <strong>Database backup.</strong> All vault, letter, and signal state lives
        in PostgreSQL. The operator backs up the <code>postgres_data</code> volume.
      </li>
      <li>
        <strong>Encryption-key backup.</strong> Letter contents are encrypted at rest
        under <code>kms.key</code>, held only on this instance. If it is lost, every
        sealed letter becomes permanently unrecoverable.
      </li>
    </ol>
    <p class="dim small">
      A backup of the database without <code>kms.key</code> — or vice versa — is not
      enough. Back up both, together and off-server. See the self-hosting guide.
    </p>
  </Card>
</div>

<style>
  .page { display: flex; flex-direction: column; gap: var(--sp-5); }

  .head { text-align: center; margin-bottom: var(--sp-3); }
  h1 { font-size: var(--size-display-2); margin: var(--sp-1) 0; }
  .lede {
    font-family: var(--font-display);
    color: var(--slate);
    font-size: 20px;
    max-width: 600px;
    margin: var(--sp-2) auto 0;
  }

  .canary-statement {
    background: var(--vellum);
    border-left: 4px solid var(--verdigris);
    margin: var(--sp-2) 0;
    padding: var(--sp-2) var(--sp-3);
    font-family: var(--font-display);
    font-size: var(--size-body-1);
    line-height: 1.6;
    color: var(--ink);
  }
  .canary-meta {
    display: flex;
    gap: var(--sp-4);
    flex-wrap: wrap;
    font-size: var(--size-body-2);
    margin-bottom: var(--sp-2);
  }

  .log-table-wrap { overflow-x: auto; margin-top: var(--sp-2); }
  .log-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--size-body-2);
  }
  .log-table th,
  .log-table td {
    padding: 8px 12px;
    border-bottom: var(--rule);
    text-align: left;
    vertical-align: top;
  }
  .log-table thead th {
    font-family: var(--font-display);
    color: var(--slate);
    font-weight: 600;
    font-size: var(--size-caption);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .kind-pill {
    display: inline-block;
    font-family: var(--font-mono, monospace);
    font-size: var(--size-caption);
    background: var(--vellum);
    border: var(--rule);
    padding: 2px 6px;
    color: var(--ink);
  }

  .protocol-list {
    padding-left: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin: var(--sp-2) 0;
  }
  .protocol-list li {
    color: var(--slate);
    line-height: 1.6;
  }
  .protocol-list strong { color: var(--ink); }

  .mono { font-family: var(--font-mono, monospace); }
  .nowrap { white-space: nowrap; }
  .small { font-size: var(--size-caption); }
  .dim { color: var(--slate); }
</style>
