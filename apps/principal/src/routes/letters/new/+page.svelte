<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { base } from '$app/paths';
  import { listVaults, uploadLetter, sealLetter, exportLetter, listVaultContacts, ApiError } from '$lib/api';
  import type { Vault, LetterWithAttachments, VaultContact } from '$lib/api';
  import { fmtBytes } from '$lib/format';
  import { LETTER_TEMPLATES, findTemplate } from '$lib/templates';
  import Card from '$lib/components/Card.svelte';
  import Field from '$lib/components/Field.svelte';
  import Textarea from '$lib/components/Textarea.svelte';
  import Button from '$lib/components/Button.svelte';
  import Banner from '$lib/components/Banner.svelte';
  import MediaRecorder from '$lib/components/MediaRecorder.svelte';

  let vaultId = $state(page.url.searchParams.get('vault') ?? '');
  let vaults = $state<Vault[]>([]);
  let vaultContacts = $state<VaultContact[]>([]);

  let title = $state('');
  let recipientEmails = $state<string[]>(['']);
  let body = $state('');
  let drillBody = $state('');
  let scheduledReleaseAt = $state('');
  let files = $state<File[]>([]);

  let templateId = $state<string>('blank');
  let kind = $state<string>('MESSAGE');
  let releaseMode = $state<'SIGNAL_OR_SCHEDULED' | 'SCHEDULED_ONLY' | 'SIGNAL_ONLY'>(
    'SIGNAL_OR_SCHEDULED'
  );
  let recordMode = $state<'off' | 'audio' | 'video'>('off');

  // Minimum datetime for scheduled release picker — now in local ISO format.
  let minDatetime = $derived(
    new Date(Date.now() - new Date().getTimezoneOffset() * 60000).toISOString().slice(0, 16)
  );
  let localTimezone = $derived(Intl.DateTimeFormat().resolvedOptions().timeZone);

  function onRecorded(file: File) {
    files = [...files, file];
    if (file.type.startsWith('video/')) kind = 'VIDEO_MESSAGE';
    else if (file.type.startsWith('audio/')) kind = 'AUDIO_MESSAGE';
  }

  let submitting = $state(false);
  let error = $state<string | null>(null);

  interface RecipientResult {
    email: string;
    letter: LetterWithAttachments | null;
    error: string | null;
  }
  let results = $state<RecipientResult[]>([]);

  $effect(() => {
    listVaults()
      .then((v) => {
        vaults = v;
        if (!vaultId && v.length) vaultId = v[0]?.id ?? '';
      })
      .catch((e) => {
        error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
      });
  });

  // Load vault contacts whenever the selected vault changes.
  $effect(() => {
    if (!vaultId) { vaultContacts = []; return; }
    listVaultContacts(vaultId)
      .then((c) => { vaultContacts = c; })
      .catch(() => { vaultContacts = []; });
  });

  function addRecipient() {
    recipientEmails = [...recipientEmails, ''];
  }

  function removeRecipient(i: number) {
    recipientEmails = recipientEmails.filter((_, idx) => idx !== i);
  }

  function applyTemplate(id: string) {
    templateId = id;
    if (id === 'blank') {
      kind = 'MESSAGE';
      releaseMode = 'SIGNAL_OR_SCHEDULED';
      return;
    }
    const tpl = findTemplate(id);
    if (!tpl) return;
    kind = tpl.kind;
    if (tpl.releaseMode) releaseMode = tpl.releaseMode;
    if (!title || confirm('Replace your draft with this template?')) {
      title = tpl.title;
      body = tpl.body;
    }
  }

  function onFiles(e: Event) {
    const input = e.target as HTMLInputElement;
    files = input.files ? Array.from(input.files) : [];
  }

  async function doExport() {
    const first = results.find((r) => r.letter);
    if (!first?.letter) return;
    const passphrase = window.prompt(
      'Choose an export passphrase (12+ characters). You will need it to open the bundle. There is no recovery if you lose it.'
    );
    if (!passphrase) return;
    if (passphrase.length < 12) {
      alert('Passphrase must be at least 12 characters.');
      return;
    }
    try {
      const bundle = await exportLetter(vaultId, first.letter.id, passphrase);
      const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `paschal-letter-${first.letter.id}.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } catch (e) {
      error = e instanceof ApiError ? e.problem.detail || e.problem.title : String(e);
    }
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (!vaultId) { error = 'Pick a Vault first.'; return; }
    if (releaseMode === 'SCHEDULED_ONLY' && !scheduledReleaseAt) {
      error = 'A time-capsule (SCHEDULED_ONLY) needs a date to open on.';
      return;
    }
    const emails = recipientEmails.map((s) => s.trim()).filter(Boolean);
    if (!emails.length) { error = 'Add at least one recipient.'; return; }

    submitting = true;
    error = null;
    results = [];

    const category = templateId === 'blank' ? undefined : templateId;
    const scheduledForApi = releaseMode === 'SIGNAL_ONLY' ? undefined : (scheduledReleaseAt || undefined);
    const scheduledRfc = scheduledForApi ? new Date(scheduledForApi).toISOString() : undefined;

    for (const email of emails) {
      try {
        let letter: LetterWithAttachments;
        if (files.length > 0) {
          letter = await uploadLetter(vaultId, {
            title,
            recipient_email: email,
            body: body || undefined,
            files,
            kind,
            category,
            release_mode: releaseMode,
            scheduled_release_at: scheduledRfc
          });
        } else {
          const sealed = await sealLetter(vaultId, {
            title,
            recipient_email: email,
            body,
            drill_body: drillBody || undefined,
            scheduled_release_at: scheduledRfc,
            kind,
            category,
            release_mode: releaseMode
          });
          letter = { ...sealed, attachments: [] };
        }
        results = [...results, { email, letter, error: null }];
      } catch (err) {
        const msg = err instanceof ApiError ? err.problem.detail || err.problem.title : String(err);
        results = [...results, { email, letter: null, error: msg }];
      }
    }
    submitting = false;
  }

  const timeCapsuleIds = $derived(
    LETTER_TEMPLATES.filter((t) => t.releaseMode === 'SCHEDULED_ONLY').map((t) => t.id)
  );
</script>

<div class="page">
  {#if results.length > 0}
    <div class="page narrow">
      <Card eyebrow="SEALED" title={title}>
        {#each results as r (r.email)}
          {#if r.letter}
            <Banner kind="ok">
              Sealed for <strong>{r.email}</strong>.
            </Banner>
          {:else}
            <Banner kind="warn">
              Failed for <strong>{r.email}</strong>: {r.error}
            </Banner>
          {/if}
        {/each}
        {#if results.some((r) => r.letter?.attachments?.length)}
          <h3 style="margin-top: var(--sp-3)">Attachments</h3>
          <ul class="att">
            {#each results.find((r) => r.letter)?.letter?.attachments ?? [] as a (a.id)}
              <li>
                <div class="name">{a.original_filename}</div>
                <div class="dim small">
                  {fmtBytes(a.original_size)} → {fmtBytes(a.transformed_size)}, {a.transformed_mime}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="row" style="margin-top: var(--sp-3)">
          <Button href={`${base}/vaults/${vaultId}`}>Back to Vault</Button>
          <a class="text-link" href="{base}/letters/new?vault={vaultId}">Seal another</a>
          {#if results.some((r) => r.letter)}
            <button type="button" class="text-link" onclick={doExport}>
              Export portable copy
            </button>
          {/if}
        </div>
      </Card>
    </div>
  {:else}
    <div class="page-head">
      <div class="eyebrow">NEW LETTER</div>
      <h1>Seal a Letter</h1>
    </div>

    <div class="compose-layout">
      <!-- Left: template picker -->
      <div class="template-panel">
        <Card eyebrow="TEMPLATES">
          {#snippet children()}
            <ul class="template-list">
              <li>
                <button
                  type="button"
                  class="tpl"
                  class:active={templateId === 'blank'}
                  onclick={() => applyTemplate('blank')}
                >
                  <div class="t-name">Blank</div>
                  <div class="t-desc">Start from scratch.</div>
                </button>
              </li>

              {#if LETTER_TEMPLATES.some((t) => !timeCapsuleIds.includes(t.id))}
                <li class="template-sep">Letters</li>
                {#each LETTER_TEMPLATES.filter((t) => !timeCapsuleIds.includes(t.id)) as t (t.id)}
                  <li>
                    <button
                      type="button"
                      class="tpl"
                      class:active={templateId === t.id}
                      onclick={() => applyTemplate(t.id)}
                    >
                      <div class="t-name">{t.name}</div>
                      <div class="t-desc">{t.description}</div>
                      {#if t.recommendedFor}
                        <div class="t-tier">Best on {t.recommendedFor}+</div>
                      {/if}
                    </button>
                  </li>
                {/each}
              {/if}

              {#if timeCapsuleIds.length}
                <li class="template-sep">Time Capsules</li>
                {#each LETTER_TEMPLATES.filter((t) => timeCapsuleIds.includes(t.id)) as t (t.id)}
                  <li>
                    <button
                      type="button"
                      class="tpl"
                      class:active={templateId === t.id}
                      onclick={() => applyTemplate(t.id)}
                    >
                      <div class="t-name">{t.name}</div>
                      <div class="t-desc">{t.description}</div>
                    </button>
                  </li>
                {/each}
              {/if}
            </ul>
          {/snippet}
        </Card>
      </div>

      <!-- Right: compose form -->
      <div>
        <Card eyebrow="COMPOSE">
          {#snippet children()}
            <form onsubmit={submit}>
              <div class="field">
                <label for="vault">Vault</label>
                <select id="vault" bind:value={vaultId} required>
                  <option value="" disabled>Choose a Vault…</option>
                  {#each vaults as v (v.id)}
                    <option value={v.id}>{v.name}</option>
                  {/each}
                </select>
              </div>

              <Field bind:value={title} label="Title" name="title" required placeholder="For Alex" />

              <!-- Vault contacts datalist -->
              {#if vaultContacts.length > 0}
                <datalist id="vault-contacts-list">
                  {#each vaultContacts as c (c.id)}
                    <option value={c.email}>{c.display_name}</option>
                  {/each}
                </datalist>
              {/if}

              <!-- Multi-recipient fields -->
              <div class="field">
                <label>Recipients <span style="color: var(--burgundy)">*</span></label>
                {#each recipientEmails as email, i (i)}
                  <div class="recipient-row">
                    <Field
                      bind:value={recipientEmails[i]}
                      label=""
                      name="recipient_email_{i}"
                      type="email"
                      required
                      placeholder="email@example.com"
                      list={vaultContacts.length > 0 ? 'vault-contacts-list' : undefined}
                    />
                    {#if recipientEmails.length > 1}
                      <button
                        type="button"
                        class="remove-btn"
                        onclick={() => removeRecipient(i)}
                        aria-label="Remove recipient"
                      >×</button>
                    {/if}
                  </div>
                {/each}
                <button type="button" class="text-link" onclick={addRecipient}>
                  + Add recipient
                </button>
                {#if recipientEmails.length > 1}
                  <p class="help">Each recipient is a separate Letter and counts against your quota.</p>
                {/if}
              </div>

              <Textarea
                bind:value={body}
                label="Body"
                name="body"
                rows={12}
                placeholder="What you want them to know."
                help="Plain text. The recipient sees exactly this. Replace any «placeholder» markers from a template."
              />

              <div class="field">
                <label>Record a voice or video message (optional)</label>
                <div class="rec-tabs">
                  <button type="button" class:active={recordMode === 'off'} onclick={() => recordMode = 'off'}>Off</button>
                  <button type="button" class:active={recordMode === 'audio'} onclick={() => recordMode = 'audio'}>Audio</button>
                  <button type="button" class:active={recordMode === 'video'} onclick={() => recordMode = 'video'}>Video</button>
                </div>
                {#if recordMode === 'audio'}
                  <MediaRecorder mode="audio" onclip={onRecorded} />
                {:else if recordMode === 'video'}
                  <MediaRecorder mode="video" onclip={onRecorded} />
                {/if}
                {#if files.some(f => f.type.startsWith('audio/'))}
                  <p class="help ok-text">Voice message attached — it will be sealed with this Letter.</p>
                {/if}
              </div>

              <div class="field">
                <label for="files">{files.length > 0 ? 'Attachments' : 'Attach files (optional)'}</label>
                <input id="files" type="file" multiple onchange={onFiles} />
                <p class="help">
                  Each file is flattened and metadata-stripped before sealing.
                  Images become JPEG, EXIF discarded; text becomes UTF-8 with LF endings;
                  PDFs are header-validated; audio has ID3 tags stripped.
                </p>
                {#if files.length > 0}
                  <ul class="filelist">
                    {#each files as f (f.name)}
                      <li>{f.name} · {fmtBytes(f.size)} · {f.type || 'binary'}</li>
                    {/each}
                  </ul>
                {/if}
              </div>

              <div class="field">
                <label>When should this Letter fire?</label>
                <div class="mode-grid">
                  <label class="mode" class:active={releaseMode === 'SIGNAL_OR_SCHEDULED'}>
                    <input type="radio" name="release_mode" checked={releaseMode === 'SIGNAL_OR_SCHEDULED'} onchange={() => (releaseMode = 'SIGNAL_OR_SCHEDULED')} />
                    <div>
                      <div class="mode-name">Whichever comes first</div>
                      <div class="mode-desc dim">Fires when signals say I'm gone — OR on the scheduled date, if one is set.</div>
                    </div>
                  </label>
                  <label class="mode" class:active={releaseMode === 'SCHEDULED_ONLY'}>
                    <input type="radio" name="release_mode" checked={releaseMode === 'SCHEDULED_ONLY'} onchange={() => (releaseMode = 'SCHEDULED_ONLY')} />
                    <div>
                      <div class="mode-name">Time-capsule — only on the date</div>
                      <div class="mode-desc dim">Fires <strong>only</strong> on the scheduled date. For wedding, graduation, or milestone letters.</div>
                    </div>
                  </label>
                  <label class="mode" class:active={releaseMode === 'SIGNAL_ONLY'}>
                    <input type="radio" name="release_mode" checked={releaseMode === 'SIGNAL_ONLY'} onchange={() => (releaseMode = 'SIGNAL_ONLY')} />
                    <div>
                      <div class="mode-name">Only when I'm gone</div>
                      <div class="mode-desc dim">Fires only when signals trigger a release. Any date is ignored.</div>
                    </div>
                  </label>
                </div>
              </div>

              {#if releaseMode !== 'SIGNAL_ONLY'}
                <Field
                  bind:value={scheduledReleaseAt}
                  label={releaseMode === 'SCHEDULED_ONLY' ? 'Open on this date' : 'Scheduled release (optional)'}
                  name="scheduled_release_at"
                  type="datetime-local"
                  min={minDatetime}
                  help={releaseMode === 'SCHEDULED_ONLY'
                    ? `Required. Fires at this time in your local timezone (${localTimezone}).`
                    : `Optional. Fires at this time in your local timezone (${localTimezone}).`}
                />
              {/if}

              {#if files.length === 0 && releaseMode !== 'SCHEDULED_ONLY'}
                <Textarea
                  bind:value={drillBody}
                  label="Drill body (optional)"
                  name="drill_body"
                  rows={4}
                  placeholder="What rehearsal recipients see in a Drill."
                  help="If absent, drills show the real body — which is fine while you're testing."
                />
              {/if}

              {#if error}<Banner kind="warn">{error}</Banner>{/if}

              <div class="row">
                <Button type="submit" disabled={submitting || !title || !recipientEmails.some(e => e.trim())}>
                  {submitting ? 'Sealing…' : recipientEmails.filter(e => e.trim()).length > 1 ? `Seal ${recipientEmails.filter(e => e.trim()).length} Letters` : 'Seal Letter'}
                </Button>
                <a class="text-link" href={vaultId ? `${base}/vaults/${vaultId}` : `${base}/dashboard`}>
                  Cancel
                </a>
              </div>
            </form>
          {/snippet}
        </Card>
      </div>
    </div>
  {/if}
</div>

<style>
  .page-head { margin-bottom: var(--sp-4); }
  h1 { margin: 4px 0 0; font-size: var(--size-display-2); }

  .compose-layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: var(--sp-4);
    align-items: start;
  }
  .template-panel {
    position: sticky;
    top: var(--sp-4);
    max-height: calc(100vh - 140px);
    overflow-y: auto;
  }
  .template-list { list-style: none; padding: 0; margin: 0; }
  .template-list li { display: block; }
  .tpl {
    display: block;
    width: 100%;
    text-align: left;
    padding: 10px var(--sp-2);
    background: none;
    border: none;
    border-bottom: var(--rule);
    cursor: pointer;
    font-family: inherit;
  }
  .tpl:hover { background: var(--bone); }
  .tpl.active { background: var(--ink); color: var(--parchment); }
  .tpl.active .t-desc { color: rgba(244, 236, 216, 0.7); }
  .t-name { font-weight: 600; font-size: var(--size-body-2); }
  .t-desc { font-size: var(--size-caption); color: var(--slate); margin-top: 2px; }
  .tpl.active .t-desc { color: rgba(244, 236, 216, 0.7); }
  .t-tier { font-size: var(--size-micro); color: var(--amber); margin-top: 3px; }
  .template-sep {
    font-size: var(--size-micro);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--slate);
    padding: var(--sp-2) var(--sp-2) 4px;
    border-bottom: var(--rule);
    pointer-events: none;
  }

  @media (max-width: 720px) {
    .compose-layout { grid-template-columns: 1fr; }
    .template-panel { position: static; max-height: none; overflow-y: visible; }
  }

  .field { margin-bottom: var(--sp-3); }
  .help { font-size: var(--size-caption); color: var(--slate); margin: 6px 0 0; }
  .ok-text { color: var(--verdigris); }
  .row { display: flex; gap: var(--sp-3); align-items: center; }
  .text-link { color: var(--slate); text-decoration: underline; text-underline-offset: 4px; cursor: pointer; background: none; border: none; font-family: inherit; font-size: inherit; padding: 0; }
  .text-link:hover { color: var(--ink); }
  .att { list-style: none; padding: 0; margin: var(--sp-2) 0 0; }
  .att li { background: var(--bone); border: 1px solid var(--mist); padding: var(--sp-2); margin-bottom: var(--sp-1); }
  .att .name { font-weight: 600; }
  .small { font-size: var(--size-caption); }
  .notes { font-size: var(--size-caption); color: var(--slate); margin: 6px 0 0; padding-left: 18px; }
  .filelist { font-size: var(--size-caption); color: var(--slate); margin-top: 6px; padding-left: 18px; }

  .recipient-row { display: flex; align-items: flex-end; gap: var(--sp-1); margin-bottom: var(--sp-1); }
  .recipient-row :global(.field) { flex: 1; margin-bottom: 0; }
  .remove-btn {
    background: none;
    border: 1px solid var(--mist);
    color: var(--slate);
    cursor: pointer;
    padding: 8px 12px;
    font-size: var(--size-body-2);
    line-height: 1;
    flex-shrink: 0;
  }
  .remove-btn:hover { border-color: var(--burgundy); color: var(--burgundy); }

  .rec-tabs { display: inline-flex; border: 1px solid var(--mist); margin-bottom: var(--sp-2); }
  .rec-tabs button { background: transparent; border: none; padding: 6px 14px; font-family: inherit; font-size: var(--size-body-2); cursor: pointer; color: var(--slate); border-right: 1px solid var(--mist); }
  .rec-tabs button:last-child { border-right: none; }
  .rec-tabs button.active { background: var(--ink); color: var(--parchment); }

  .mode-grid { display: grid; grid-template-columns: 1fr; gap: var(--sp-2); margin-top: 6px; }
  .mode { display: grid; grid-template-columns: 24px 1fr; gap: 10px; align-items: flex-start; padding: var(--sp-2); border: 1px solid var(--mist); background: var(--vellum); cursor: pointer; }
  .mode.active { border-color: var(--burgundy); box-shadow: 0 0 0 2px var(--burgundy); }
  .mode input { margin-top: 4px; }
  .mode-name { font-weight: 600; }
  .mode-desc { font-size: var(--size-caption); margin-top: 4px; }
</style>
