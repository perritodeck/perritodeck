<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { HostStatus, PairingOffer } from "../lib/types";

  let { status, ondone }: { status: HostStatus; ondone: () => void } = $props();

  let offer = $state<PairingOffer | null>(null);
  let error = $state<string | null>(null);
  let expiresAt = $state(0);
  let left = $state(0);
  let paired = $state<string | null>(null);

  async function fresh() {
    error = null;
    paired = null;
    try {
      offer = await api.startPairing();
      expiresAt = Date.now() + offer.expiresInSecs * 1000;
      left = offer.expiresInSecs;
    } catch (e) {
      error = String(e);
    }
  }

  const clock = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`);
  const expired = $derived(offer != null && left <= 0);

  onMount(() => {
    fresh();
    const t = setInterval(() => (left = Math.max(0, Math.round((expiresAt - Date.now()) / 1000))), 500);
    const off = api.onHostEvent((e) => {
      if (e.kind === "paired") paired = e.deviceName;
    });
    return () => {
      clearInterval(t);
      off.then((f) => f());
    };
  });
</script>

<h1>Pair a tablet</h1>

{#if error}
  <p class="note" role="alert">{error}</p>
{:else if paired}
  <section class="done" aria-live="polite">
    <h2>Paired with {paired}</h2>
    <p>It will connect by itself from now on, whenever this computer is on the same network.</p>
    <div class="row">
      <button class="btn primary" onclick={ondone}>See tablets</button>
      <button class="btn" onclick={fresh}>Pair another</button>
    </div>
  </section>
{:else if offer}
  <div class="pair">
    <figure class="qr" class:stale={expired} aria-label="QR code for pairing">
      <!-- The SVG comes from the host (rendered by the qrcode crate). -->
      {@html offer.qrSvg}
    </figure>

    <div class="steps">
      <ol>
        <li>Open <strong>Perrito</strong> on the tablet and tap <em>Scan QR code</em>, or point the tablet’s camera app here.</li>
        <li>
          No camera? Pick <em>{status.hostName}</em> in Perrito and type this code:
          <span class="code selectable" aria-label="Pairing code {offer.code.split('').join(' ')}"
            >{offer.code.slice(0, 3)}<span class="gap"></span>{offer.code.slice(3)}</span
          >
        </li>
        <li>
          Check that the tablet shows this fingerprint:
          <span class="fp selectable">{offer.fingerprint}</span>
        </li>
      </ol>

      <p class="timer" aria-live="polite">
        {#if expired}
          This code has expired.
        {:else}
          Waiting for the tablet. This code works for {clock}.
        {/if}
      </p>
      <button class="btn" class:primary={expired} onclick={fresh}>New code</button>
    </div>
  </div>
{/if}

<style>
  h1 { margin-bottom: 24px; }

  .pair {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 40px;
    align-items: start;
    max-width: 820px;
  }

  .qr {
    margin: 0;
    width: 260px;
    padding: 14px;
    background: #fff;
    border-radius: 22px;
    box-shadow: 0 0 0 1px var(--line);
    transition: opacity 200ms;
  }
  .qr :global(svg) { display: block; width: 100%; height: auto; }
  .qr.stale { opacity: 0.25; }

  ol {
    margin: 0 0 20px;
    padding-left: 1.3em;
    display: grid;
    gap: 16px;
  }
  li::marker { font: 700 1rem var(--display); color: var(--apricot-deep); }

  .code {
    display: block;
    margin-top: 8px;
    font: 750 3.4rem/1 var(--display);
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
  }
  .gap { display: inline-block; width: 0.4em; }

  .fp {
    display: inline-block;
    margin-left: 4px;
    padding: 1px 8px;
    border-radius: 6px;
    background: var(--paper);
    border: 1px solid var(--line);
    font-family: ui-monospace, monospace;
    font-size: 0.92rem;
  }

  .timer { color: var(--ink-soft); margin-bottom: 12px; font-variant-numeric: tabular-nums; }

  .done {
    display: grid;
    gap: 12px;
    max-width: 52ch;
  }
  .done h2 { font-size: 1.6rem; }
  .row { display: flex; gap: 10px; margin-top: 6px; }
  .note { max-width: 60ch; }

  @media (max-width: 860px) {
    .pair { grid-template-columns: 1fr; }
  }
</style>
