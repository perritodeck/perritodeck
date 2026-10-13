<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api } from "../lib/api";
  import type { DeviceInfo, ProfileSummary } from "../lib/types";
  import FirewallHint from "../components/FirewallHint.svelte";

  let { devices, onchange, onpair }: { devices: DeviceInfo[]; onchange: () => void; onpair: () => void } = $props();

  let profiles = $state<ProfileSummary[]>([]);
  let confirming = $state<string | null>(null);
  let renaming = $state<string | null>(null);
  let nameDraft = $state("");
  let error = $state<string | null>(null);

  const defaultId = $derived(profiles.find((p) => p.isDefault)?.id ?? "default");
  const nameOf = (id: string) => profiles.find((p) => p.id === id)?.name ?? id;

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

  async function act(f: () => Promise<unknown>) {
    try {
      await f();
      error = null;
      onchange();
    } catch (e) {
      error = String(e);
    }
  }

  async function unpair(id: string) {
    await act(() => api.revoke(id));
    confirming = null;
  }

  async function startRename(d: DeviceInfo) {
    renaming = d.id;
    nameDraft = d.name;
    await tick();
    document.querySelector<HTMLInputElement>("#device-name")?.select();
  }

  async function saveName(id: string) {
    await act(() => api.renameDevice(id, nameDraft));
    renaming = null;
  }

  onMount(async () => {
    profiles = await api.profiles();
  });
</script>

<header>
  <h1>Tablets</h1>
  <button class="btn primary" onclick={onpair}>Pair a tablet</button>
</header>

<FirewallHint />

{#if error}
  <p class="err" role="alert">{error}</p>
{/if}

{#if devices.length === 0}
  <p class="empty">No tablets yet. Pair one to put your buttons on it.</p>
{:else}
  <ul>
    {#each devices as d (d.id)}
      <li>
        <span class="dot" class:on={d.connected} aria-hidden="true"></span>
        <div class="who">
          {#if renaming === d.id}
            <form class="rename" onsubmit={(e) => (e.preventDefault(), saveName(d.id))}>
              <input
                id="device-name"
                aria-label="Tablet name"
                bind:value={nameDraft}
                maxlength="40"
                onkeydown={(e) => e.key === "Escape" && (renaming = null)}
              />
              <button class="btn small primary" type="submit">Save</button>
            </form>
          {:else}
            <button class="name" title="Rename" onclick={() => startRename(d)}>{d.name}</button>
          {/if}
          <span>
            {seen(d)}{#if d.connected && d.profile !== (d.assignedProfile ?? defaultId)}, showing {nameOf(d.profile)} for now{/if}
          </span>
        </div>
        {#if profiles.length > 1}
          <label class="shows">
            <span>Shows</span>
            <select
              value={d.assignedProfile ?? defaultId}
              onchange={(e) => act(() => api.assignProfile(d.id, e.currentTarget.value === defaultId ? null : e.currentTarget.value))}
            >
              {#each profiles as p (p.id)}
                <option value={p.id}>{p.name}</option>
              {/each}
            </select>
          </label>
        {/if}
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
    max-width: 860px;
  }
  .empty { color: var(--ink-soft); }
  .err { color: var(--danger); margin-bottom: 12px; }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-width: 860px;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
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
    min-width: 200px;
    justify-items: start;
  }
  .who > span { color: var(--ink-soft); font-size: 0.9rem; }
  .name {
    border: 0;
    background: none;
    padding: 0;
    font-weight: 700;
    text-align: left;
  }
  .name:hover { text-decoration: underline; text-decoration-style: dotted; }
  .rename { display: flex; gap: 6px; }
  .rename input,
  .shows select {
    font: inherit;
    color: var(--ink);
    background: var(--paper);
    border: 1.5px solid var(--line);
    border-radius: 8px;
    padding: 4px 8px;
  }
  .shows { display: flex; align-items: center; gap: 8px; color: var(--ink-soft); font-size: 0.9rem; }
  .btn.small { padding: 4px 12px; font-size: 0.85rem; }
  .confirm {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .confirm span { color: var(--ink-soft); font-size: 0.9rem; margin-right: 4px; }
</style>
