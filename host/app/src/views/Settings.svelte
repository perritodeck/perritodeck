<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { HostStatus } from "../lib/types";

  let { status, onchange }: { status: HostStatus; onchange: () => void } = $props();
  let error = $state<string | null>(null);
  let autostart = $state<boolean | null>(null);

  onMount(async () => {
    try {
      autostart = await api.autostart();
    } catch {
      autostart = null;
    }
  });

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
  <dd>{status.version}</dd>
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
</style>
