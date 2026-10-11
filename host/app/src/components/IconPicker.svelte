<script lang="ts">
  import { ICON_NAMES, iconSvg } from "../lib/icons";

  let { color, onpick, onfile, onclose }: { color: string; onpick: (name: string) => void; onfile: () => void; onclose: () => void } =
    $props();

  let query = $state("");
  const shown = $derived(
    ICON_NAMES.filter((n) => n.includes(query.trim().toLowerCase().replaceAll(" ", "-"))).slice(0, 120),
  );
</script>

<div class="icon-picker" role="dialog" aria-label="Choose an image">
  <div class="top">
    <!-- svelte-ignore a11y_autofocus -->
    <input aria-label="Search icons" placeholder="Search: mic, light, play…" bind:value={query} autofocus onkeydown={(e) => e.key === "Escape" && onclose()} />
    <button type="button" class="btn small" onclick={onfile}>From a file…</button>
  </div>
  <div class="grid">
    {#each shown as name (name)}
      <button type="button" class="cell" title={name.replaceAll("-", " ")} aria-label={name.replaceAll("-", " ")} onclick={() => onpick(name)}>
        {@html iconSvg(name, color, 22)}
      </button>
    {:else}
      <p class="none">No icon matches “{query}”.</p>
    {/each}
  </div>
  <p class="credit">Icons from Tabler Icons (MIT).</p>
</div>

<style>
  .icon-picker {
    display: grid;
    gap: 8px;
    padding: 10px;
    border-radius: 12px;
    background: var(--paper);
    border: 1px solid var(--line);
    box-shadow: 0 12px 30px -12px rgb(0 0 0 / 0.35);
  }
  .top { display: flex; gap: 6px; }
  .top input {
    flex: 1;
    font: inherit;
    padding: 5px 8px;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    background: var(--fog);
    color: var(--ink);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(34px, 1fr));
    gap: 4px;
    max-height: 210px;
    overflow: auto;
  }
  .cell {
    aspect-ratio: 1;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 8px;
    background: var(--deck);
  }
  .cell:hover,
  .cell:focus-visible { outline: 2px solid var(--apricot); }
  .none, .credit { color: var(--ink-soft); font-size: 0.8rem; margin: 0; }
  .btn.small { padding: 4px 10px; font-size: 0.85rem; }
</style>
