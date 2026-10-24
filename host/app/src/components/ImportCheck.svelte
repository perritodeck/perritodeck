<script lang="ts">
  // Asks before adding a profile whose keys run things. Everything shown comes from the
  // file, already made safe by the host: invisible and direction-changing characters
  // arrive as marks, never as themselves.
  import { onMount } from "svelte";
  import type { ImportChoice, ImportPreview, RiskItem, RiskyKey, Shown } from "../lib/types";

  let { preview, busy, onchoose }: { preview: ImportPreview; busy: boolean; onchoose: (choice: ImportChoice | null) => void } =
    $props();

  let dialog = $state<HTMLDialogElement | null>(null);
  let list = $state<HTMLDivElement | null>(null);
  let cancelButton = $state<HTMLButtonElement | null>(null);
  let safe = $state<HTMLButtonElement | null>(null);
  let everything = $state<HTMLButtonElement | null>(null);
  // Set when this component closes the dialog itself, so that close isn't a cancel.
  let closing = false;

  const one = $derived(preview.total === 1);
  const groups = $derived.by(() => {
    const out: { page: Shown; pageNumber: number; keys: RiskyKey[] }[] = [];
    for (const k of preview.risky) {
      const last = out[out.length - 1];
      if (last && last.pageNumber === k.pageNumber) last.keys.push(k);
      else out.push({ page: k.page, pageNumber: k.pageNumber, keys: [k] });
    }
    return out;
  });
  const intro = $derived(
    preview.keysThatRun && preview.keysThatType
      ? "Pressing them runs programs or plugins, or types text and presses Enter."
      : preview.keysThatType
        ? "Pressing them types text and presses Enter. In a terminal or the Run box, that runs a command."
        : "Pressing them runs programs or plugins on this computer.",
  );

  function choose(choice: ImportChoice | null) {
    if (!busy) onchoose(choice);
  }

  function keys(n: number) {
    return `${n} ${n === 1 ? "key" : "keys"}`;
  }

  function and(ns: number[]) {
    return ns.length > 1 ? `${ns.slice(0, -1).join(", ")} and ${ns[ns.length - 1]}` : `${ns[0]}`;
  }

  /** Where in the key it is: "State 2 of 3 · step 1 of 2". */
  function placeOf(item: RiskItem, stateCount: number) {
    const parts: string[] = [];
    if (item.unused) parts.push("not used by any state now");
    else if (item.states.length === 1) parts.push(`state ${item.states[0]} of ${stateCount}`);
    else if (item.states.length > 1) parts.push(`states ${and(item.states)} of ${stateCount}`);
    for (const p of item.place) parts.push(p.in === "step" ? `step ${p.n} of ${p.of}` : `choice ${p.n} of ${p.of}, picked at random`);
    const s = parts.join(" · ");
    return s ? s[0].toUpperCase() + s.slice(1) : "";
  }

  onMount(() => {
    const d = dialog!;
    if (typeof d.showModal === "function") d.showModal();
    else {
      d.setAttribute("open", "");
      d.classList.add("fallback");
    }
    safe?.focus();
    // Keep Tab inside, in a fixed order: the list, then the buttons.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const order: HTMLElement[] = [];
      for (const el of [list, cancelButton, safe, everything]) if (el) order.push(el);
      const i = order.indexOf(document.activeElement as HTMLElement);
      e.preventDefault();
      const next = i < 0 ? (e.shiftKey ? order.length - 1 : 0) : (i + (e.shiftKey ? order.length - 1 : 1)) % order.length;
      order[next].focus();
    };
    d.addEventListener("keydown", onKey);
    return () => {
      d.removeEventListener("keydown", onKey);
      closing = true;
      if (d.open) d.close();
    };
  });
</script>

{#snippet value(s: Shown, cls = "v")}
  <code dir="ltr" class="{cls} selectable"
    >{#each s as piece, i (i)}{#if typeof piece === "string"}{piece}{:else if "mark" in piece}{#if piece.mark === "Enter"}<span
            class="mark">⏎<span class="sr-only">line break</span></span
          ><br />{:else if piece.mark === "Tab"}<span class="mark">⇥<span class="sr-only">tab</span></span>{:else}<span
            class="mark"
            title="An invisible character">{piece.mark}</span
          >{/if}{:else}<span class="cut">…and {piece.more} more characters not shown</span>{/if}{/each}</code
  >
{/snippet}

{#snippet plain(s: Shown)}
  {#each s as piece, i (i)}{#if typeof piece === "string"}{piece}{:else if "mark" in piece}<span class="mark"
        >{piece.mark === "Enter" ? "⏎" : piece.mark === "Tab" ? "⇥" : piece.mark}</span
      >{:else}…{/if}{/each}
{/snippet}

{#snippet args(list: Shown[], more: number)}
  {#each list as a, i (i)}
    {#if a.length}{@render value(a, "v arg")}{:else}<code class="v arg empty">""</code>{/if}
  {/each}
  {#if more}<span class="cut">…and {more} more arguments</span>{/if}
{/snippet}

{#snippet plugin(w: { plugin: Shown; action: Shown; settings: Shown | null; installed: boolean; pluginName: Shown | null; actionName: Shown | null })}
  {@render value(w.pluginName ?? w.plugin)}: {@render value(w.actionName ?? w.action)}
  {#if w.settings}with {@render value(w.settings)}{/if}
  {#if !w.installed}<span class="note">(not installed, so it does nothing until you add it)</span>{/if}
{/snippet}

<dialog
  bind:this={dialog}
  class="import-check"
  aria-labelledby="import-check-title"
  aria-describedby="import-check-intro"
  oncancel={(e) => {
    e.preventDefault();
    choose(null);
  }}
  onclose={() => {
    // Chromium may close a modal dialog on Escape without a cancel event.
    if (!closing) choose(null);
  }}
>
  <header>
    <h2 id="import-check-title">
      “{@render plain(preview.name)}” {one ? "has a key that runs things" : "has keys that run things"}
    </h2>
    <p class="from">From {@render plain(preview.file)}</p>
    <p id="import-check-intro">
      {intro} They can do anything you can, so keep them only if you trust whoever made this file.
    </p>
    <p class="count">
      {preview.total} of {preview.keys} keys{preview.pages > 1 ? `, on ${preview.pagesWithRisky} of ${preview.pages} pages` : ""}:
    </p>
  </header>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="list" role="region" aria-label="Keys that run things" tabindex="0" bind:this={list}>
    {#each groups as g (g.pageNumber)}
      <h3>Page “{@render plain(g.page)}” · {keys(g.keys.length)}</h3>
      <ul class="keys">
        {#each g.keys as k (`${k.row},${k.col}`)}
          <li>
            <strong>{#if k.title.length}{@render plain(k.title)}{:else}No title{/if}</strong>
            <span class="at">row {k.row + 1}, column {k.col + 1}</span>
            <ul class="items">
              {#each k.items as item, i (i)}
                {@const where = placeOf(item, k.stateCount)}
                {@const w = item.what}
                <li>
                  {#if where}<span class="where">{where}</span>{/if}
                  <span class="line">
                    {#if w.type === "command"}
                      Runs {@render value(w.program)} {@render args(w.args, w.moreArgs)}
                      {#if w.cwd}in {@render value(w.cwd)}{/if}
                    {:else if w.type === "launch"}
                      Starts {@render value(w.program)} {@render args(w.args, w.moreArgs)}
                    {:else if w.type === "commandSlider"}
                      Each move of the slider runs {@render value(w.program)} {@render args(w.args, w.moreArgs)}
                    {:else if w.type === "openPath"}
                      Opens {@render value(w.path)} with its app
                    {:else if w.type === "plugin"}
                      Uses the plugin {@render plugin(w)}
                    {:else if w.type === "pluginSlider"}
                      Each move of the slider uses the plugin {@render plugin(w)}
                    {:else if w.type === "sdPlugin"}
                      Uses the Stream Deck plugin {@render plugin(w)}
                      {#if w.onShow}<span class="note">It gets this key’s settings as soon as a tablet shows its page.</span>{/if}
                    {:else if w.type === "typing"}
                      {w.enterInText ? "Types this (⏎ is Enter):" : "Types this, then another step presses Enter:"}
                      {@render value(w.text, "v block")}
                    {/if}
                  </span>
                </li>
              {/each}
            </ul>
            {#if k.moreItems}<p class="more">…and {k.moreItems} more in this key</p>{/if}
          </li>
        {/each}
      </ul>
    {/each}
    {#if preview.moreKeys}<p class="more">…and {preview.moreKeys} more keys not listed here.</p>{/if}
  </div>

  <footer>
    <p class="hint">
      {one
        ? "Import with this key off keeps how it looks but removes what’s listed, so pressing it does nothing."
        : "Import with these keys off keeps how they look but removes what’s listed, so pressing them does nothing."}{preview.hasSliders
        ? " Sliders become plain keys."
        : ""}
    </p>
    <div class="buttons">
      <button class="btn quiet" bind:this={cancelButton} aria-disabled={busy || undefined} onclick={() => choose(null)}>Cancel</button>
      <button class="btn primary" bind:this={safe} aria-disabled={busy || undefined} onclick={() => choose("switchOff")}
        >{one ? "Import with this key off" : "Import with these keys off"}</button
      >
      <button class="btn danger" bind:this={everything} aria-disabled={busy || undefined} onclick={() => choose("everything")}
        >Import everything</button
      >
    </div>
  </footer>
</dialog>

<style>
  .import-check {
    width: min(720px, calc(100vw - 32px));
    max-width: none;
    max-height: calc(100vh - 48px);
    padding: 20px 24px;
    background: var(--paper);
    color: var(--ink);
    border: 1px solid var(--line);
    border-top: 4px solid var(--danger);
    border-radius: 12px;
  }
  .import-check[open] {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .import-check::backdrop {
    background: rgb(0 0 0 / 0.5);
  }
  .import-check:global(.fallback) {
    position: fixed;
    inset: 24px 0 auto;
    margin: 0 auto;
    z-index: 100;
    box-shadow: 0 0 0 100vmax rgb(0 0 0 / 0.5);
  }
  header,
  footer {
    flex: none;
  }
  h2 {
    margin: 0;
    font-family: var(--display);
    font-size: 1.25rem;
  }
  header p {
    margin: 6px 0 0;
  }
  .from {
    color: var(--ink-soft);
    font-size: 0.875rem;
    overflow-wrap: anywhere;
  }
  .count {
    font-weight: 700;
  }
  .list {
    flex: 1 1 auto;
    min-height: 120px;
    overflow: auto;
    background: var(--fog);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px 12px;
  }
  .list:focus-visible {
    outline: 2px solid var(--apricot-deep);
    outline-offset: 2px;
  }
  h3 {
    margin: 10px 0 4px;
    font-size: 1rem;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .keys > li {
    padding: 6px 0;
    border-bottom: 1px solid var(--line);
  }
  .keys > li:last-child {
    border-bottom: none;
  }
  .at {
    color: var(--ink-soft);
    margin-left: 6px;
  }
  .items > li {
    margin: 4px 0 0 12px;
    overflow-wrap: anywhere;
  }
  .where {
    display: block;
    font-size: 0.8125rem;
    color: var(--ink-soft);
  }
  .v {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 0.875rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: isolate;
    background: var(--paper);
    border-radius: 4px;
    padding: 0 4px;
  }
  .v.arg {
    border: 1px solid var(--line);
    margin-right: 4px;
  }
  .v.block {
    display: block;
    margin-top: 2px;
  }
  .mark {
    display: inline-block;
    border: 1px solid var(--danger);
    color: var(--danger);
    border-radius: 4px;
    padding: 0 3px;
    margin: 0 1px;
    font-size: 0.75rem;
    line-height: 1.3;
  }
  .cut,
  .note,
  .more {
    color: var(--ink-soft);
    font-style: italic;
  }
  .note {
    display: block;
  }
  .more {
    margin: 6px 0 0;
  }
  .hint {
    margin: 0 0 12px;
    color: var(--ink-soft);
    font-size: 0.875rem;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }
  .btn[aria-disabled="true"] {
    opacity: 0.6;
    cursor: progress;
  }
</style>
