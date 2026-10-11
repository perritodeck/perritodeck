<script lang="ts">
  import { api } from "../lib/api";
  import type { HostStatus } from "../lib/types";

  let { status }: { status: HostStatus } = $props();
  let error = $state<string | null>(null);

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

  <dt>Profile</dt>
  <dd>
    <span class="mono selectable">{status.profilePath}</span>
    <span><button class="btn" onclick={() => run(api.openProfileFile)}>Edit profile file</button></span>
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
</style>
