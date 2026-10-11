<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { HostStatus, Network, Update } from "../lib/types";

  let { status, onchange }: { status: HostStatus; onchange: () => void } = $props();
  let error = $state<string | null>(null);
  let autostart = $state<boolean | null>(null);
  // What's saved, and what's being edited.
  let network = $state<Network | null>(null);
  let port = $state("");
  let lan = $state(true);
  let needsRestart = $state(false);
  let checkUpdates = $state<boolean | null>(null);
  let checking = $state(false);
  let checked = $state<{ update: Update | null } | null>(null);

  onMount(async () => {
    try {
      autostart = await api.autostart();
    } catch {
      autostart = null;
    }
    try {
      network = await api.network();
      port = String(network.port);
      lan = network.lan;
      checkUpdates = await api.checkUpdatesEnabled();
    } catch {
      // Older host or mock without these: hide the rows.
    }
  });

  const portNumber = $derived(/^\d+$/.test(port.trim()) ? Number(port.trim()) : NaN);
  const portProblem = $derived(
    Number.isInteger(portNumber) && portNumber >= 1024 && portNumber <= 65535 ? null : "Use a number from 1024 to 65535.",
  );
  const networkChanged = $derived(!!network && (portNumber !== network.port || lan !== network.lan));

  function saveNetwork() {
    if (portProblem) return;
    run(async () => {
      const next = { port: portNumber, lan };
      await api.setNetwork(next);
      network = next;
      needsRestart = true;
    });
  }

  function checkNow() {
    checking = true;
    checked = null;
    run(async () => {
      try {
        checked = { update: await api.checkForUpdates() };
      } finally {
        checking = false;
      }
    });
  }

  async function run(f: () => Promise<void>) {
    error = null;
    try {
      await f();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<h1>Settings</h1>

{#if error}
  <p class="err" role="alert">{error}</p>
{/if}

<dl>
  <dt>Keys</dt>
  <dd>
    <label class="check">
      <input
        type="checkbox"
        checked={status.paused}
        onchange={(e) => run(async () => (await api.setPaused(e.currentTarget.checked), onchange()))}
      />
      Pause all keys
    </label>
    <small>While paused, presses on the tablets do nothing. Also in the tray menu.</small>
  </dd>

  {#if autostart !== null}
    <dt>Start</dt>
    <dd>
      <label class="check">
        <input
          type="checkbox"
          checked={autostart}
          onchange={(e) => {
            const on = e.currentTarget.checked;
            run(async () => {
              await api.setAutostart(on);
              autostart = on;
            });
          }}
        />
        Start PerritoDeck when I log in
      </label>
      <small>It starts in the tray, so the tablets work without opening this window.</small>
    </dd>
  {/if}

  {#if network}
    <dt>Network</dt>
    <dd>
      <div class="row">
        <label for="port">Port</label>
        <input
          id="port"
          class="port"
          inputmode="numeric"
          bind:value={port}
          aria-invalid={!!portProblem}
          aria-describedby="port-help"
        />
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={lan} />
        Let tablets connect over Wi-Fi
      </label>
      <small id="port-help">
        {#if portProblem && port.trim() !== ""}
          <span class="bad">{portProblem}</span>
        {:else if !lan}
          Only tablets on a USB cable can connect. Tablets on Wi-Fi will lose their connection.
        {:else}
          Change the port only if another program uses {network.port}. Tablets on the same Wi-Fi find the new port by
          themselves; if you typed an address into Perrito, change it there too.
        {/if}
      </small>
      {#if networkChanged}
        <span><button class="btn" onclick={saveNetwork} disabled={!!portProblem}>Save network settings</button></span>
      {/if}
      {#if needsRestart && !networkChanged}
        <div class="restart" role="status">
          <span>Saved. PerritoDeck uses the new settings after it restarts.</span>
          <button class="btn small" onclick={() => run(api.restart)}>Restart now</button>
        </div>
      {/if}
    </dd>
  {/if}

  <dt>This computer</dt>
  <dd>{status.hostName}</dd>

  <dt>Fingerprint</dt>
  <dd>
    <span class="mono selectable">{status.fingerprint ?? "none (development mode)"}</span>
    <small>Tablets check this when pairing, so nobody else can pretend to be this computer.</small>
  </dd>

  <dt>Tablets connect to</dt>
  <dd>
    {#if status.addresses.length}
      {#each status.addresses as a (a)}<span class="mono selectable">{a}</span>{/each}
    {:else}
      <span>No network found</span>
    {/if}
    <small>Most tablets find this computer by themselves. Type an address in Perrito only if yours doesn’t.</small>
  </dd>

  <dt>USB</dt>
  <dd>
    {#if status.usbAvailable}
      <span>
        {status.usbDevices.length
          ? `${status.usbDevices.length} tablet${status.usbDevices.length === 1 ? "" : "s"} on a cable`
          : "No tablet on a cable right now"}
      </span>
      <small>
        Tablets plugged in with USB debugging on connect over the cable, which is steadier than Wi-Fi. Pair them as
        usual; they switch to the cable by themselves.
      </small>
    {:else}
      <span>Not set up</span>
      <small>
        To use a tablet over a USB cable, install Android’s platform-tools (it includes <code>adb</code>) and restart
        PerritoDeck.
      </small>
    {/if}
  </dd>

  <dt>Profiles</dt>
  <dd>
    <span class="mono selectable">{status.profilesDir}</span>
    <small>One <code>.toml</code> file per profile. Files you add, change or delete here show up straight away.</small>
  </dd>

  <dt>Settings folder</dt>
  <dd>
    <span class="mono selectable">{status.dataDir}</span>
    <small>Holds paired tablets, this computer’s certificate and <code>integrations.toml</code> for OBS and Home Assistant.</small>
    <span><button class="btn" onclick={() => run(api.openDataFolder)}>Open folder</button></span>
  </dd>

  <dt>Integrations</dt>
  <dd>
    {status.integrations.length ? status.integrations.join(", ") : "None set up"}
  </dd>

  <dt>Version</dt>
  <dd>
    <span>{status.version}</span>
    {#if checkUpdates !== null}
      <label class="check">
        <input
          type="checkbox"
          checked={checkUpdates}
          onchange={(e) => {
            const on = e.currentTarget.checked;
            run(async () => {
              await api.setCheckUpdates(on);
              checkUpdates = on;
            });
          }}
        />
        Tell me when a new version is out
      </label>
      <small>PerritoDeck asks GitHub when it starts. It never downloads or installs anything by itself.</small>
      <span class="row">
        <button class="btn" onclick={checkNow} disabled={checking}>{checking ? "Checking…" : "Check now"}</button>
        {#if checked}
          {#if checked.update}
            {@const u = checked.update}
            <span role="status">Version {u.version} is out.</span>
            <button class="btn quiet" onclick={() => run(() => api.openReleasePage(u.url))}>See what's new</button>
          {:else}
            <span role="status">You have the latest version.</span>
          {/if}
        {/if}
      </span>
    {/if}
  </dd>
</dl>

<style>
  h1 { margin-bottom: 24px; }
  dl {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr);
    gap: 22px 24px;
    max-width: 780px;
    margin: 0;
  }
  dt { color: var(--ink-soft); font-weight: 700; }
  dd {
    margin: 0;
    display: grid;
    gap: 6px;
    justify-items: start;
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.9rem; overflow-wrap: anywhere; }
  small { color: var(--ink-soft); font-size: 0.85rem; max-width: 60ch; }
  .err { color: var(--danger); margin-bottom: 16px; }
  .check { display: flex; align-items: center; gap: 8px; }
  .check input { width: 17px; height: 17px; accent-color: var(--apricot-deep); }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .port {
    width: 8ch;
    font: inherit;
    font-family: ui-monospace, monospace;
    padding: 5px 8px;
    border: 1.5px solid var(--line, #ccc);
    border-radius: 8px;
    background: var(--paper);
    color: var(--ink);
  }
  .port[aria-invalid="true"] { border-color: var(--danger); }
  .bad { color: var(--danger); }
  .restart {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--apricot) 28%, var(--paper));
  }
</style>
