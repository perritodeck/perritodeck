<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import type { DeviceInfo, HostStatus, Update, View } from "./lib/types";
  import Buttons from "./views/Buttons.svelte";
  import Tablets from "./views/Tablets.svelte";
  import Pair from "./views/Pair.svelte";
  import Settings from "./views/Settings.svelte";
  import Switching from "./views/Switching.svelte";
  import Plugins from "./views/Plugins.svelte";

  let view = $state<View>("buttons");
  /** Where in Settings to open, when another view sends the person there. */
  let anchor = $state<string | null>(null);
  let status = $state<HostStatus | null>(null);
  let startError = $state<string | null>(null);
  let devices = $state<DeviceInfo[]>([]);
  // macOS: false until PerritoDeck is allowed to send key presses.
  let accessibility = $state<boolean | null>(null);

  async function checkAccessibility() {
    try {
      accessibility = await api.accessibilityAllowed();
    } catch {
      accessibility = null;
    }
  }

  let update = $state<Update | null>(null);

  async function lookForUpdate() {
    try {
      if (await api.checkUpdatesEnabled()) update = await api.checkForUpdates();
    } catch {
      // Offline, or no releases yet: nothing to say.
    }
  }

  const connected = $derived(devices.filter((d) => d.connected).length);

  async function refreshDevices() {
    devices = await api.devices();
  }

  async function refreshStatus() {
    status = await api.status();
  }

  async function resume() {
    await api.setPaused(false);
    await refreshStatus();
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
    checkAccessibility();
    lookForUpdate();
    const onFocus = () => checkAccessibility();
    window.addEventListener("focus", onFocus);
    offs.push(Promise.resolve(() => window.removeEventListener("focus", onFocus)));
    offs.push(
      api.onHostEvent((e) => {
        if (e.kind === "connected" || e.kind === "disconnected" || e.kind === "paired") refreshDevices();
        if (e.kind === "profileChanged") {
          refreshStatus();
          refreshDevices();
        }
      }),
    );
    return () => offs.forEach((p) => p.then((off) => off()));
  });

  const nav: { id: View; label: string }[] = [
    { id: "buttons", label: "Buttons" },
    { id: "tablets", label: "Tablets" },
    { id: "pair", label: "Pair a tablet" },
    { id: "switching", label: "Switching" },
    { id: "plugins", label: "Plugins" },
    { id: "settings", label: "Settings" },
  ];
</script>

<div class="shell">
  <nav class="rail" aria-label="Sections">
    <div class="brand">
      <!-- The app icon: docs/assets/icon-master-rounded.svg's shapes. -->
      <svg viewBox="0 0 1024 1024" width="30" height="30" aria-hidden="true">
        <rect x="72" y="72" width="880" height="880" rx="200" fill="#1E1E2E" />
        <g transform="translate(512 512) scale(0.86) translate(-512 -510)">
          <rect x="337" y="474" width="350" height="296" rx="104" fill="#D9875B" />
          <rect x="337" y="474" width="350" height="274" rx="104" fill="#FAB387" />
          <rect x="362" y="250" width="124" height="140" rx="46" fill="#D9875B" />
          <rect x="362" y="250" width="124" height="126" rx="46" fill="#FAB387" />
          <rect x="538" y="250" width="124" height="140" rx="46" fill="#D9875B" />
          <rect x="538" y="250" width="124" height="126" rx="46" fill="#FAB387" />
          <rect x="200" y="380" width="124" height="140" rx="46" fill="#D9875B" transform="rotate(-18 262 450)" />
          <rect x="200" y="380" width="124" height="126" rx="46" fill="#FAB387" transform="rotate(-18 262 450)" />
          <rect x="700" y="380" width="124" height="140" rx="46" fill="#D9875B" transform="rotate(18 762 450)" />
          <rect x="700" y="380" width="124" height="126" rx="46" fill="#FAB387" transform="rotate(18 762 450)" />
        </g>
      </svg>
      <span>PerritoDeck</span>
    </div>
    <ul>
      {#each nav as item (item.id)}
        <li>
          <button
            class:active={view === item.id}
            aria-current={view === item.id ? "page" : undefined}
            onclick={() => {
              anchor = null;
              view = item.id;
            }}
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
        <h1>PerritoDeck couldn't start</h1>
        <p class="selectable">{startError}</p>
        <p>
          If another PerritoDeck is already running (the app or <code>perritodeck-host</code> in a terminal),
          quit it and open PerritoDeck again.
        </p>
      </section>
    {:else if status}
      {#if accessibility === false}
        <div class="paused" role="alert">
          <span
            ><strong>Hotkeys and typing need your permission.</strong> In System Settings, under Privacy &amp; Security →
            Accessibility, turn on PerritoDeck.</span
          >
          <button class="btn small" onclick={() => api.openAccessibilitySettings().then(checkAccessibility)}>Open System Settings</button>
        </div>
      {/if}
      {#if update}
        <div class="paused" role="status">
          <span><strong>PerritoDeck {update.version} is out.</strong> You have {status.version}.</span>
          <span class="actions">
            <button class="btn small" onclick={() => update && api.openReleasePage(update.url)}>See what's new</button>
            <button class="btn small quiet" onclick={() => (update = null)}>Not now</button>
          </span>
        </div>
      {/if}
      {#if status.paused}
        <div class="paused" role="status">
          <span><strong>Keys are paused.</strong> Presses on the tablets do nothing until you resume.</span>
          <button class="btn small" onclick={resume}>Resume</button>
        </div>
      {/if}
      {#if view === "buttons"}
        <Buttons
          {status}
          onsetup={(id) => {
            anchor = id;
            view = "settings";
          }}
        />
      {:else if view === "tablets"}
        <Tablets {devices} onchange={refreshDevices} onpair={() => (view = "pair")} />
      {:else if view === "pair"}
        <Pair {status} ondone={() => (view = "tablets")} />
      {:else if view === "plugins"}
        <Plugins />
      {:else if view === "switching"}
        <Switching {status} {devices} />
      {:else}
        <Settings {status} {anchor} onchange={refreshStatus} />
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

  .paused {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    max-width: 980px;
    margin: -12px 0 20px;
    padding: 10px 10px 10px 16px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--apricot) 28%, var(--paper));
  }
  .paused .btn { padding: 5px 14px; }
  .actions { display: flex; gap: 6px; flex: none; }

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
