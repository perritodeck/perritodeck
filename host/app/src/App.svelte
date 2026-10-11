<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import type { DeviceInfo, HostStatus, View } from "./lib/types";
  import Buttons from "./views/Buttons.svelte";
  import Tablets from "./views/Tablets.svelte";
  import Pair from "./views/Pair.svelte";
  import Settings from "./views/Settings.svelte";

  let view = $state<View>("buttons");
  let status = $state<HostStatus | null>(null);
  let startError = $state<string | null>(null);
  let devices = $state<DeviceInfo[]>([]);

  const connected = $derived(devices.filter((d) => d.connected).length);

  async function refreshDevices() {
    devices = await api.devices();
  }

  onMount(() => {
    const offs: Promise<() => void>[] = [];
    (async () => {
      try {
        status = await api.status();
        await refreshDevices();
        if (devices.length === 0) view = "pair";
      } catch (e) {
        startError = String(e);
      }
    })();
    offs.push(api.onNavigate((v) => (view = v)));
    offs.push(
      api.onHostEvent((e) => {
        if (e.kind === "connected" || e.kind === "disconnected" || e.kind === "paired") refreshDevices();
      }),
    );
    return () => offs.forEach((p) => p.then((off) => off()));
  });

  const nav: { id: View; label: string }[] = [
    { id: "buttons", label: "Buttons" },
    { id: "tablets", label: "Tablets" },
    { id: "pair", label: "Pair a tablet" },
    { id: "settings", label: "Settings" },
  ];
</script>

<div class="shell">
  <nav class="rail" aria-label="Sections">
    <div class="brand">
      <svg viewBox="0 0 48 48" width="30" height="30" aria-hidden="true">
        <rect x="2" y="2" width="44" height="44" rx="10" fill="#1E1E2E" />
        <path fill="#FAB387" d="M24 24c-5.5 0-10 4.5-10 8.5 0 3 2.5 4.5 5 4.5 2 0 3.2-1 5-1s3 1 5 1c2.5 0 5-1.5 5-4.5 0-4-4.5-8.5-10-8.5z" />
        <ellipse cx="14" cy="22" rx="3.2" ry="3.6" fill="#FAB387" />
        <ellipse cx="20" cy="15" rx="3.2" ry="3.8" fill="#FAB387" />
        <ellipse cx="28" cy="15" rx="3.2" ry="3.8" fill="#FAB387" />
        <ellipse cx="34" cy="22" rx="3.2" ry="3.6" fill="#FAB387" />
      </svg>
      <span>PerritoDeck</span>
    </div>
    <ul>
      {#each nav as item (item.id)}
        <li>
          <button
            class:active={view === item.id}
            aria-current={view === item.id ? "page" : undefined}
            onclick={() => (view = item.id)}
            disabled={!!startError}
          >
            {item.label}
            {#if item.id === "tablets" && devices.length > 0}
              <span class="count" aria-label="{connected} connected">{connected}/{devices.length}</span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
    {#if status}
      <p class="host" title="This computer">
        <span class="dot" class:on={connected > 0}></span>
        {status.hostName}
      </p>
    {/if}
  </nav>

  <main>
    {#if startError}
      <section class="failed" role="alert">
        <h1>The host didn't start</h1>
        <p class="selectable">{startError}</p>
        <p>
          If another PerritoDeck is already running (the app or <code>perritodeck-host</code> in a terminal),
          quit it and open PerritoDeck again.
        </p>
      </section>
    {:else if status}
      {#if view === "buttons"}
        <Buttons {status} />
      {:else if view === "tablets"}
        <Tablets {devices} onchange={refreshDevices} onpair={() => (view = "pair")} />
      {:else if view === "pair"}
        <Pair {status} ondone={() => (view = "tablets")} />
      {:else}
        <Settings {status} />
      {/if}
    {/if}
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: 208px minmax(0, 1fr);
    height: 100%;
  }

  .rail {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding: 20px 14px;
    border-right: 1px solid var(--line);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 8px;
    font: 750 1.1rem/1 var(--display);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }

  li button {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: center;
    text-align: left;
    padding: 9px 12px;
    border: 0;
    border-radius: 10px;
    background: none;
    color: var(--ink-soft);
    font-weight: 700;
  }
  li button:hover:not(:disabled) { color: var(--ink); background: var(--paper); }
  li button.active { color: var(--ink); background: var(--paper); box-shadow: inset 3px 0 0 var(--apricot); }
  li button:disabled { opacity: 0.4; cursor: default; }

  .count { font-weight: 400; font-size: 0.85rem; font-variant-numeric: tabular-nums; }

  .host {
    margin-top: auto;
    padding: 0 12px;
    font-size: 0.85rem;
    color: var(--ink-soft);
    display: flex;
    align-items: center;
    gap: 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--ink-soft);
  }
  .dot.on { background: var(--apricot); border-color: var(--apricot); }

  main {
    overflow: auto;
    padding: 32px 40px 48px;
  }

  .failed {
    max-width: 60ch;
    display: grid;
    gap: 14px;
  }
  .failed p:first-of-type {
    font-family: ui-monospace, monospace;
    font-size: 0.9rem;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--paper);
    border: 1px solid var(--line);
  }
</style>
