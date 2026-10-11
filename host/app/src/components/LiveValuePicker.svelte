<script lang="ts" module>
  let counter = 0;
</script>

<script lang="ts">
  import type { PluginManifest } from "../lib/types";

  let {
    plugins = [],
    label = "Insert a live value",
    onpick,
  }: {
    plugins?: PluginManifest[];
    label?: string;
    /** The value's name, without braces, e.g. `plugin:dev.perritodeck.uptime:up:nas`. */
    onpick: (name: string) => void;
  } = $props();

  const uid = `lv${++counter}`;
  const builtIn = [
    { name: "time", label: "Time (21:07)" },
    { name: "time_s", label: "Time with seconds" },
    { name: "date", label: "Date (Sun 11 Oct)" },
    { name: "cpu", label: "This computer's CPU use" },
    { name: "mem", label: "This computer's memory use" },
    { name: "obs:scene", label: "OBS: current scene (built-in)" },
    { name: "obs:recording", label: "OBS: recording (built-in)" },
    { name: "obs:streaming", label: "OBS: streaming (built-in)" },
  ];

  let open = $state(false);
  let choice = $state("");
  let arg = $state("");
  const picked = $derived.by(() => {
    if (!choice.startsWith("plugin:")) return null;
    const [, pid, vid] = choice.split(":");
    const p = plugins.find((x) => x.id === pid);
    return p?.values.find((v) => v.id === vid) ?? null;
  });
  const ready = $derived(!!choice && (!picked?.arg || arg.trim() !== ""));

  function insert() {
    if (!ready) return;
    onpick(picked?.arg ? `${choice}:${arg.trim()}` : choice);
    open = false;
    choice = "";
    arg = "";
  }
</script>

{#if !open}
  <button type="button" class="link" onclick={() => (open = true)}>{label}…</button>
{:else}
  <div class="picker" role="group" aria-label={label}>
    <label class="sr-only" for="{uid}-v">Live value</label>
    <select id="{uid}-v" bind:value={choice}>
      <option value="" disabled>Choose a value…</option>
      <optgroup label="Built in">
        {#each builtIn as b (b.name)}<option value={b.name}>{b.label}</option>{/each}
      </optgroup>
      {#each plugins.filter((p) => p.values.length) as p (p.id)}
        <optgroup label={p.name}>
          {#each p.values as v (v.id)}<option value={`plugin:${p.id}:${v.id}`}>{v.name}</option>{/each}
        </optgroup>
      {/each}
    </select>
    {#if picked?.arg}
      <label class="sr-only" for="{uid}-a">{picked.arg}</label>
      <input id="{uid}-a" bind:value={arg} placeholder={picked.arg} spellcheck="false" onkeydown={(e) => e.key === "Enter" && (e.preventDefault(), insert())} />
    {/if}
    <button type="button" class="btn small" disabled={!ready} onclick={insert}>Insert</button>
    <button type="button" class="btn small quiet" onclick={() => (open = false)}>Cancel</button>
  </div>
{/if}

<style>
  .picker { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .picker select, .picker input { flex: 1 1 140px; min-width: 0; }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--apricot-deep);
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
    text-decoration: underline;
    justify-self: start;
  }
</style>
