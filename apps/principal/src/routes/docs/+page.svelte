<script lang="ts">
  import { base } from '$app/paths';
  import Card from '$lib/components/Card.svelte';
</script>

<svelte:head>
  <title>How Paschal works</title>
</svelte:head>

<div class="page narrow">
  <header class="page-head">
    <div class="eyebrow">DOCUMENTATION</div>
    <h1>How Paschal works</h1>
    <p class="lede">
      A plain-language guide to the vault, the signal system, encryption, and what happens
      if we can no longer operate.
    </p>
  </header>

  <Card eyebrow="THE BASICS" title="What Paschal does">
    {#snippet children()}
      <p>
        Paschal is a dead-man's switch for digital estates. You write letters, seal them in an
        encrypted Vault, and name a Recipient for each one. The letters stay sealed until the
        signal system determines you are no longer reachable — then a cooling-off window opens,
        and you can cancel it at any time before delivery happens.
      </p>
      <p>The core flow has four steps:</p>
      <ol>
        <li><strong>Author.</strong> Write a Letter (text, voice message, credentials, instructions, playlist). Attach files. Seal it into a Vault against a named Recipient.</li>
        <li><strong>Signal.</strong> Keep the signal system satisfied — send a Heartbeat, have a Buddy confirm you're well, let your bank automation ping the webhook. As long as signals say you're present, nothing moves.</li>
        <li><strong>Cooling-off.</strong> When signals suggest absence, a configurable cooling-off window opens (14 days by default). A notification is sent to you. If you're reachable, cancel it. If you're not, it elapses and delivery begins.</li>
        <li><strong>Delivery.</strong> Each Letter is sent to its named Recipient at their email address. Attachments are included. The transparency log records a hash of the event — it cannot be silently undone.</li>
      </ol>
    {/snippet}
  </Card>

  <Card eyebrow="ENCRYPTION" title="How your data is protected">
    {#snippet children()}
      <p>
        Every Letter is encrypted with <strong>AES-256-GCM</strong> before it is stored.
        A unique key is generated per Letter. Keys are managed via a KMS abstraction
        (<code>crypto-stub</code>) that wraps AWS KMS in production and uses a local file
        key in self-hosted deployments.
      </p>
      <h3>Two encryption tiers</h3>
      <dl>
        <dt>Honest Operator (default)</dt>
        <dd>
          Letters are encrypted at rest. The operator holds the key via KMS but
          makes a binding commitment not to read content outside of a valid
          release trigger. The transparency log makes any access auditable.
          This is the standard tier for hosted Paschal accounts.
        </dd>
        <dt>Zero Knowledge <em>(ships Q4 2026 after independent audit)</em></dt>
        <dd>
          Keys are derived from a passphrase you hold and never leave your device.
          The operator stores only ciphertext. Without your passphrase, nobody —
          including Paschal — can read your letters. Delivery requires the
          Recipient to hold a key shard.
        </dd>
      </dl>
      <p class="dim small">
        The cryptographic audit is scheduled for Q3 2026. The full report — including any
        findings and how they were resolved — will be published at
        <a href="https://paschal.app/audit" rel="external noopener">paschal.app/audit</a>.
      </p>
    {/snippet}
  </Card>

  <Card eyebrow="SIGNALS" title="How the system knows you're gone">
    {#snippet children()}
      <p>
        Paschal does not make a unilateral call. A weighted signal aggregator
        scores several independent sources. Only when the combined score exceeds
        the trigger threshold — and no single class dominates — does the
        cooling-off window open.
      </p>
      <h3>Available now</h3>
      <ul>
        <li>
          <strong>Heartbeat.</strong> A manual button in the Dashboard. Press it
          periodically to confirm you're present. The system measures how long it
          has been since the last press.
        </li>
        <li>
          <strong>Buddy attestation.</strong> Friends or family who confirm
          (or deny) your wellbeing on a schedule. Two confirmed Buddies provide
          meaningful independent signal.
        </li>
        <li>
          <strong>Apple iCloud Shortcut.</strong> A Shortcut on your iPhone or
          iPad that pings a webhook on a schedule. When the device stops pinging,
          the signal contribution rises.
        </li>
        <li>
          <strong>Bank activity webhook.</strong> A private URL you paste into
          YNAB, your bank's automation, or an Apple Shortcut. Any HTTP POST to
          the URL registers bank activity as proof of life.
        </li>
      </ul>
      <h3>Coming later</h3>
      <ul>
        <li>Calendar inactivity (Microsoft / Google)</li>
        <li>Direct bank dormancy via open banking (CDR / PSD2)</li>
      </ul>
    {/snippet}
  </Card>

  <Card eyebrow="RETENTION & DELETION" title="What happens to your data">
    {#snippet children()}
      <h3>Subscription cancellation</h3>
      <p>
        Cancelling your subscription enters a <strong>retention window</strong> (1–25 years
        depending on your plan). During retention, new authoring is blocked but existing
        Letters remain sealed and releasable on signal triggers. You can reactivate at
        any time during the window.
      </p>
      <h3>Account deletion</h3>
      <p>
        Requesting account deletion schedules permanent destruction of every Vault, Letter,
        and Attachment after a <strong>30-day cooling-off period</strong>. You can cancel
        the deletion request at any time during those 30 days. The transparency log retains
        hashes only — not the letter content.
      </p>
      <h3>What is erased</h3>
      <p>
        Encrypted letter ciphertext, attachments, and all personally-identifiable metadata
        are purged. The transparency log is append-only and retains only cryptographic hashes
        of events — it cannot be selectively edited.
      </p>
    {/snippet}
  </Card>

  <Card eyebrow="SERVICE CONTINUITY" title="If Paschal can no longer operate" id="service-continuity">
    {#snippet children()}
      <p>
        If Paschal is unable to continue operating — for any reason — we commit to
        releasing your data to you or your nominated Co-Steward as a
        <strong>secure encrypted export</strong> before the service shuts down.
      </p>
      <p>The export contains:</p>
      <ul>
        <li>All Letter ciphertext (decryptable with your account key or ZK passphrase)</li>
        <li>All Attachments in their transformed form</li>
        <li>Recipient metadata and release configuration</li>
        <li>A copy of the transparency log for your account</li>
      </ul>
      <p>
        The export format is documented in
        <code>docs/EXPORT_FORMAT.md</code> in the open-source repository.
        A standalone decryption tool ships alongside it so the export remains
        readable without Paschal infrastructure.
      </p>
      <p>
        The source code is published under AGPL. If we shut down without
        providing an export, anyone can fork the repository and self-host
        a compatible reader.
      </p>
      <p class="dim small">
        By signing up, you acknowledge this commitment and agree that
        the export represents fulfilment of Paschal's obligation to you.
      </p>
    {/snippet}
  </Card>

  <div class="nav-row">
    <a href="{base}/">← Back to home</a>
    <a href="{base}/signin">Create an account →</a>
  </div>
</div>

<style>
  .page-head { margin-bottom: var(--sp-4); }
  h1 { margin: 4px 0 var(--sp-2); font-size: var(--size-display-2); }
  .lede { font-family: var(--font-display); font-size: 20px; color: var(--slate); margin: 0; }
  h3 { font-size: var(--size-h3); margin: var(--sp-3) 0 var(--sp-1); }
  p { margin: 0 0 var(--sp-2); }
  ol, ul { padding-left: var(--sp-3); margin: 0 0 var(--sp-2); }
  li { margin-bottom: var(--sp-1); font-size: var(--size-body-2); }
  dl { margin: 0 0 var(--sp-2); }
  dt { font-weight: 600; font-size: var(--size-body-2); margin-top: var(--sp-2); }
  dd { margin: 4px 0 0 var(--sp-3); font-size: var(--size-body-2); }
  .small { font-size: var(--size-caption); }
  .nav-row {
    display: flex;
    justify-content: space-between;
    margin-top: var(--sp-4);
    font-size: var(--size-body-2);
  }
</style>
