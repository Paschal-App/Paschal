<script lang="ts">
  // A small wrapper around the browser MediaRecorder API. Captures audio
  // (and optionally video) and produces a File the parent can attach to a
  // Letter via the existing multipart pipeline.

  type Props = {
    mode: 'audio' | 'video';
    /** Called with the captured File when the user finishes a recording. */
    onclip: (file: File) => void;
    /** Optional max duration in seconds (auto-stops). Defaults to 10 min for audio, 5 for video. */
    maxSeconds?: number;
  };
  let { mode, onclip, maxSeconds }: Props = $props();

  const limit = $derived(maxSeconds ?? (mode === 'audio' ? 600 : 300));

  let stream = $state<MediaStream | null>(null);
  let recorder = $state<MediaRecorder | null>(null);
  let chunks: Blob[] = [];
  let recording = $state(false);
  let elapsed = $state(0);
  let interval: ReturnType<typeof setInterval> | null = null;
  let videoEl: HTMLVideoElement | null = $state(null);
  let lastBlobUrl = $state<string | null>(null);
  let error = $state<string | null>(null);

  function fmtTime(s: number): string {
    const m = Math.floor(s / 60);
    const r = s % 60;
    return `${m}:${r.toString().padStart(2, '0')}`;
  }

  async function start() {
    error = null;
    try {
      const constraints: MediaStreamConstraints =
        mode === 'audio' ? { audio: true } : { audio: true, video: { facingMode: 'user' } };
      stream = await navigator.mediaDevices.getUserMedia(constraints);
      if (mode === 'video' && videoEl) {
        videoEl.srcObject = stream;
        await videoEl.play().catch(() => {});
      }
      // Pick a reasonable container the browser supports.
      const candidates =
        mode === 'audio'
          ? ['audio/webm;codecs=opus', 'audio/webm', 'audio/ogg', 'audio/mp4']
          : ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm', 'video/mp4'];
      let mime = '';
      for (const c of candidates) {
        if (MediaRecorder.isTypeSupported(c)) {
          mime = c;
          break;
        }
      }
      chunks = [];
      recorder = new MediaRecorder(stream, mime ? { mimeType: mime } : {});
      recorder.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) chunks.push(e.data);
      };
      recorder.onstop = () => {
        const blob = new Blob(chunks, { type: recorder?.mimeType || mime || 'application/octet-stream' });
        if (lastBlobUrl) URL.revokeObjectURL(lastBlobUrl);
        lastBlobUrl = URL.createObjectURL(blob);
        const ext = (recorder?.mimeType || mime || '').includes('mp4') ? 'mp4' : 'webm';
        const filename = `${mode}-message-${Date.now()}.${ext}`;
        const file = new File([blob], filename, { type: blob.type });
        onclip(file);
        cleanup();
      };
      recorder.start(250);
      recording = true;
      elapsed = 0;
      interval = setInterval(() => {
        elapsed += 1;
        if (elapsed >= limit) stop();
      }, 1000);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function stop() {
    if (recorder && recorder.state !== 'inactive') {
      recorder.stop();
    }
    recording = false;
    if (interval) { clearInterval(interval); interval = null; }
  }

  function cleanup() {
    stream?.getTracks().forEach((t) => t.stop());
    stream = null;
    if (videoEl) videoEl.srcObject = null;
    recorder = null;
  }

  function discard() {
    if (lastBlobUrl) {
      URL.revokeObjectURL(lastBlobUrl);
      lastBlobUrl = null;
    }
  }
</script>

<div class="recorder">
  {#if mode === 'video'}
    <video bind:this={videoEl} muted playsinline class="preview" class:active={recording}></video>
  {/if}

  {#if error}
    <p class="error">⚠ {error}</p>
    <p class="dim small">
      If your browser blocks microphone or camera access, you can still upload a pre-recorded
      file using the regular attach-file control below.
    </p>
  {/if}

  <div class="row">
    {#if !recording}
      <button type="button" class="rec" onclick={start}>
        {mode === 'audio' ? '● Record audio' : '● Record video'}
      </button>
    {:else}
      <button type="button" class="stop" onclick={stop}>■ Stop ({fmtTime(elapsed)})</button>
    {/if}
    <span class="dim small">
      Max {fmtTime(limit)}. Recorded in your browser; not uploaded until you Seal the Letter.
    </span>
  </div>

  {#if lastBlobUrl && !recording}
    <div class="playback">
      <p class="dim small">Review your recording:</p>
      {#if mode === 'audio'}
        <audio controls src={lastBlobUrl}></audio>
      {:else}
        <video controls src={lastBlobUrl} class="preview"></video>
      {/if}
      <button type="button" class="link-btn" onclick={discard}>Discard and re-record</button>
    </div>
  {/if}
</div>

<style>
  .recorder {
    border: 1px dashed var(--mist);
    background: var(--bone);
    padding: var(--sp-2);
    margin-bottom: var(--sp-2);
  }
  .row { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .preview {
    max-width: 100%;
    width: 320px;
    height: 240px;
    background: black;
    margin-bottom: var(--sp-2);
  }
  audio { width: 100%; max-width: 480px; }
  .rec, .stop {
    background: var(--ink);
    color: var(--parchment);
    border: 1px solid var(--ink);
    padding: 6px 14px;
    font-family: inherit;
    font-size: var(--size-body-2);
    cursor: pointer;
  }
  .stop { background: var(--burgundy); border-color: var(--burgundy); }
  .rec:hover { background: var(--burgundy); border-color: var(--burgundy); }
  .link-btn {
    background: none;
    border: none;
    color: var(--burgundy);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 4px;
    font: inherit;
    padding: 0;
    margin-top: 6px;
  }
  .small { font-size: var(--size-caption); }
  .error { color: var(--burgundy); margin: 0 0 6px; }
  .playback { margin-top: var(--sp-2); }
</style>
