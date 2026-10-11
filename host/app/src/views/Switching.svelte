<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { AutoSwitch, DeviceInfo, HostStatus, ProfileSummary } from "../lib/types";

  let { status, devices }: { status: HostStatus; devices: DeviceInfo[] } = $props();

  let auto = $state<AutoSwitch | null>(null);
  let profiles = $state<ProfileSummary[]>([]);
  let recent = $state<string[]>([]);
  let front = $state<string | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);

  async function save(next: AutoSwitch) {
    auto = next;
    try {
      await api.setAutoSwitch(next);
      error = null;
      saved = true;
      setTimeout(() => (saved = false), 1500);
    } catch (e) {
      error = String(e);
    }
  }

  function addRule() {
    if (!auto) return;
    const profile = profiles.find((p) => !p.isDefault)?.id ?? profiles[0]?.id ?? "default";
    save({ ...auto, enabled: true, rules: [...auto.rules, { app: recent[0] ?? "", profile, device: null }] });
  }

  function update(i: number, patch: Partial<AutoSwitch["rules"][number]>) {
    if (!auto) return;
    save({ ...auto, rules: auto.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) });
  }

  onMount(() => {
    (async () => {
      [auto, profiles, recent] = await Promise.all([api.autoSwitch(), api.profiles(), api.recentApps()]);
    })();
    const off = api.onHostEvent(async (e) => {
      if (e.kind === "foregroundApp") {
        front = e.app;
        recent = await api.recentApps();
      }
    });
    return () => off.then((f) => f());
  });
</script>

<h1>Switching</h1>
<p class="intro">Show a different profile while a particular app is in front, then go back when you leave it.</p>

{#if !status.autoSwitchSupported}
  <p class="note">
    This desktop doesn’t tell other apps which window is in front (Wayland), so switching by app isn’t available here
    yet. Choosing a profile for each tablet still works, on the Tablets page.
  </p>
{/if}

{#if error}
  <p class="err" role="alert">{error}</p>
{/if}

{#if auto}
  <label class="toggle">
    <input type="checkbox" checked={auto.enabled} onchange={(e) => save({ ...auto!, enabled: e.currentTarget.checked })} />
    <span>Switch profiles by app</span>
    {#if saved}<span class="saved" role="status">Saved</span>{/if}
  </label>

  {#if auto.rules.length}
    <table class:off={!auto.enabled}>
      <thead>
        <tr><th scope="col">When this app is in front</th><th scope="col">Show</th><th scope="col">On</th><th></th></tr>
      </thead>
      <tbody>
        {#each auto.rules as rule, i (i)}
          <tr>
            <td>
              <input
                aria-label="App name"
                list="recent-apps"
                value={rule.app}
                placeholder="obs64"
                spellcheck="false"
                onchange={(e) => update(i, { app: e.currentTarget.value.trim() })}
              />
            </td>
            <td>
              <select aria-label="Profile to show" value={rule.profile} onchange={(e) => update(i, { profile: e.currentTarget.value })}>
                {#each profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
              </select>
            </td>
            <td>
              <select
                aria-label="Which tablets"
                value={rule.device ?? ""}
                onchange={(e) => update(i, { device: e.currentTarget.value || null })}
              >
                <option value="">Every tablet</option>
                {#each devices as d (d.id)}<option value={d.id}>{d.name}</option>{/each}
              </select>
            </td>
            <td>
              <button class="btn quiet small" onclick={() => save({ ...auto!, rules: auto!.rules.filter((_, j) => j !== i) })}
                >Remove</button
              >
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
  <datalist id="recent-apps">
    {#each recent as a (a)}<option value={a}></option>{/each}
  </datalist>

  <button class="btn" onclick={addRule}>Add a rule</button>

  <p class="hint">
    {#if front}
      In front just now: <code>{front}</code>.
    {/if}
    App names are the ones listed as you type: switch to an app, come back here, and it shows up. The first matching rule
    wins.
  </p>
{/if}

<style>
  h1 { margin-bottom: 6px; }
  .intro { color: var(--ink-soft); margin-bottom: 20px; max-width: 60ch; }
  .note {
    max-width: 70ch;
    padding: 10px 14px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: var(--paper);
    margin-bottom: 18px;
  }
  .err { color: var(--danger); margin-bottom: 12px; }
  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 700;
    margin-bottom: 16px;
  }
  .toggle input { width: 18px; height: 18px; accent-color: var(--apricot-deep); }
  .saved { font-weight: 400; color: var(--ink-soft); font-size: 0.85rem; }
  table {
    border-collapse: collapse;
    width: 100%;
    max-width: 860px;
    margin-bottom: 14px;
  }
  table.off { opacity: 0.55; }
  th {
    text-align: left;
    font-size: 0.85rem;
    color: var(--ink-soft);
    font-weight: 700;
    padding: 0 8px 6px 0;
  }
  td { padding: 4px 8px 4px 0; }
  td input,
  td select {
    font: inherit;
    width: 100%;
    color: var(--ink);
    background: var(--paper);
    border: 1.5px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
  }
  .btn.small { padding: 4px 12px; font-size: 0.85rem; }
  .hint { margin-top: 18px; color: var(--ink-soft); font-size: 0.9rem; max-width: 70ch; }
</style>
