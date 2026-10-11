<script lang="ts">
  import { api } from "../lib/api";
  import type { DeviceInfo } from "../lib/types";

  let { devices, onchange, onpair }: { devices: DeviceInfo[]; onchange: () => void; onpair: () => void } = $props();

  let confirming = $state<string | null>(null);

  function seen(d: DeviceInfo): string {
    if (d.connected) return "Connected now";
    if (d.lastSeen == null) return "Hasn’t connected yet";
    const s = Math.max(0, Date.now() / 1000 - d.lastSeen);
    if (s < 60) return "Last seen just now";
    if (s < 3600) return `Last seen ${Math.floor(s / 60)} min ago`;
    if (s < 86400) return `Last seen ${Math.floor(s / 3600)} h ago`;
    const days = Math.floor(s / 86400);
    return `Last seen ${days} day${days === 1 ? "" : "s"} ago`;
  }

  async function unpair(id: string) {
    await api.revoke(id);
    confirming = null;
    onchange();
  }
</script>

<header>
  <h1>Tablets</h1>
  <button class="btn primary" onclick={onpair}>Pair a tablet</button>
</header>

{#if devices.length === 0}
  <p class="empty">No tablets yet. Pair one to put your buttons on it.</p>
{:else}
  <ul>
    {#each devices as d (d.id)}
      <li>
        <span class="dot" class:on={d.connected} aria-hidden="true"></span>
        <div class="who">
          <strong>{d.name}</strong>
          <span>{seen(d)}</span>
        </div>
        {#if confirming === d.id}
          <div class="confirm" role="group" aria-label="Unpair {d.name}?">
            <span>It will need a new code to connect again.</span>
            <button class="btn danger" onclick={() => unpair(d.id)}>Unpair</button>
            <button class="btn quiet" onclick={() => (confirming = null)}>Keep</button>
          </div>
        {:else}
          <button class="btn quiet" onclick={() => (confirming = d.id)}>Unpair…</button>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 20px;
    max-width: 760px;
  }
  .empty { color: var(--ink-soft); }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-width: 760px;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 4px;
    border-bottom: 1px solid var(--line);
    min-height: 68px;
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--ink-soft);
  }
  .dot.on { background: var(--apricot); border-color: var(--apricot); }
  .who {
    display: grid;
    flex: 1;
    min-width: 0;
  }
  .who span { color: var(--ink-soft); font-size: 0.9rem; }
  .confirm {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .confirm span { color: var(--ink-soft); font-size: 0.9rem; margin-right: 4px; }
</style>
