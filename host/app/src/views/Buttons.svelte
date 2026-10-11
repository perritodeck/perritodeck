<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { HostStatus, KeyView, ProfileView } from "../lib/types";

  let { status }: { status: HostStatus } = $props();

  let profile = $state<ProfileView | null>(null);
  let pageIndex = $state(0);
  let selected = $state<KeyView | null>(null);
  let now = $state(new Date());
  let openError = $state<string | null>(null);

  const page = $derived(profile?.pages[Math.min(pageIndex, profile.pages.length - 1)] ?? null);

  const cells = $derived.by(() => {
    if (!profile || !page) return [];
    const out: (KeyView | null)[] = [];
    for (let r = 0; r < profile.rows; r++)
      for (let c = 0; c < profile.cols; c++) out.push(page.keys.find((k) => k.row === r && k.col === c) ?? null);
    return out;
  });

  async function load() {
    profile = await api.profile();
    if (selected) {
      const s = selected;
      selected = page?.keys.find((k) => k.row === s.row && k.col === s.col) ?? null;
    }
  }

  // Live titles, as the tablet shows them.
  function live(title: string): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    return title
      .replaceAll("{time}", `${pad(now.getHours())}:${pad(now.getMinutes())}`)
      .replaceAll("{time_s}", `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`)
      .replaceAll("{date}", now.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" }));
  }

  const kindLabel: Record<KeyView["kind"], string> = {
    action: "Action",
    page: "Opens a page",
    back: "Goes back",
    next: "Next page",
    previous: "Previous page",
    toggle: "Toggle",
    label: "Label only",
  };

  function goTo(k: KeyView) {
    if (!profile) return;
    if (k.kind === "page") {
      const name = k.detail.replace(/^opens /, "");
      const i = profile.pages.findIndex((p) => p.name === name);
      if (i >= 0) {
        pageIndex = i;
        selected = null;
      }
    } else if (k.kind === "back" || k.kind === "previous") {
      pageIndex = Math.max(0, pageIndex - 1);
      selected = null;
    } else if (k.kind === "next") {
      pageIndex = Math.min(profile.pages.length - 1, pageIndex + 1);
      selected = null;
    }
  }

  async function openFile() {
    openError = null;
    try {
      await api.openProfileFile();
    } catch (e) {
      openError = String(e);
    }
  }

  onMount(() => {
    load();
    const off = api.onHostEvent((e) => {
      if (e.kind === "profileChanged") load();
    });
    const t = setInterval(() => (now = new Date()), 1000);
    return () => {
      clearInterval(t);
      off.then((f) => f());
    };
  });
</script>

<header>
  <div>
    <h1>Buttons</h1>
    {#if profile}
      <p class="sub">Profile “{profile.name}”, {profile.cols} × {profile.rows} keys</p>
    {/if}
  </div>
  <button class="btn" onclick={openFile}>Edit profile file</button>
</header>

{#if openError}
  <p class="banner" role="alert">{openError}</p>
{/if}

{#if profile?.reloadError}
  <div class="banner" role="alert">
    <strong>Your last edit wasn’t applied.</strong> The tablets still show the version before it. Fix this and save again:
    <pre class="selectable">{profile.reloadError}</pre>
  </div>
{/if}

{#if profile && page}
  {#if profile.pages.length > 1}
    <div class="pages" role="tablist" aria-label="Pages">
      {#each profile.pages as p, i (p.name)}
        <button
          role="tab"
          aria-selected={i === pageIndex}
          class:active={i === pageIndex}
          onclick={() => {
            pageIndex = i;
            selected = null;
          }}>{p.name}</button
        >
      {/each}
    </div>
  {/if}

  <div class="layout">
    <div class="slab" style="--cols: {profile.cols}; --rows: {profile.rows}">
      {#each cells as k, i (i)}
        {#if k}
          <button
            class="key"
            class:selected={selected === k}
            style="background: {k.bg}; color: {k.fg}"
            aria-label="{live(k.title).replace('\n', ' ')}: {k.detail}"
            aria-pressed={selected === k}
            onclick={() => (selected = k)}
            ondblclick={() => goTo(k)}
          >
            <span>{live(k.title)}</span>
          </button>
        {:else}
          <div class="key empty" aria-hidden="true"></div>
        {/if}
      {/each}
    </div>

    <aside class="detail" aria-live="polite">
      {#if selected}
        <p class="where">Row {selected.row + 1}, column {selected.col + 1}</p>
        <h2>{live(selected.title).replace("\n", " ") || "Untitled"}</h2>
        <dl>
          <dt>Kind</dt>
          <dd>{kindLabel[selected.kind]}</dd>
          <dt>Does</dt>
          <dd class="selectable">{selected.detail}</dd>
        </dl>
        {#if ["page", "back", "next", "previous"].includes(selected.kind)}
          <button class="btn" onclick={() => selected && goTo(selected)}>Follow it</button>
        {/if}
      {:else}
        <p class="hint">Select a key to see what it does. Double-click a page key to follow it.</p>
        <p class="hint">
          To change buttons, edit the profile file; the tablets update as soon as you save. A drag-and-drop editor is on
          the way.
        </p>
      {/if}
    </aside>
  </div>
{/if}

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    margin-bottom: 20px;
  }
  .sub { color: var(--ink-soft); margin-top: 4px; }

  .banner {
    margin-bottom: 18px;
    padding: 12px 16px;
    border-radius: 12px;
    border: 1.5px solid var(--danger);
    background: var(--paper);
    max-width: 760px;
  }
  .banner pre {
    margin: 8px 0 0;
    white-space: pre-wrap;
    font-size: 0.85rem;
  }

  .pages {
    display: flex;
    gap: 6px;
    margin-bottom: 14px;
  }
  .pages button {
    padding: 5px 14px;
    border-radius: 999px;
    border: 1.5px solid var(--line);
    background: none;
    color: var(--ink-soft);
    font-weight: 700;
  }
  .pages button.active { background: var(--ink); border-color: var(--ink); color: var(--fog); }

  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 250px;
    gap: 28px;
    align-items: start;
  }

  /* The tablet, drawn as it looks: a dark slab with rounded keys. */
  .slab {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: clamp(6px, 1.2vw, 12px);
    padding: clamp(12px, 2vw, 22px);
    border-radius: 26px;
    background: var(--deck);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--deck) 70%, #000), 0 18px 40px -18px rgb(0 0 0 / 0.45);
    max-width: 760px;
  }

  .key {
    aspect-ratio: 1;
    border: 0;
    border-radius: 14%;
    display: grid;
    place-items: center;
    padding: 6%;
    font: 700 clamp(0.62rem, 1.15vw, 0.95rem)/1.15 var(--display);
    text-align: center;
    white-space: pre-line;
    overflow: hidden;
    transition: transform 80ms ease-out;
  }
  .key:hover { transform: translateY(-1px); }
  .key:active { transform: scale(0.96); }
  .key.selected { outline: 3px solid var(--apricot); outline-offset: 3px; }
  .key.empty { background: color-mix(in srgb, var(--deck) 82%, #fff); opacity: 0.35; }

  .detail {
    display: grid;
    gap: 10px;
    padding-top: 4px;
  }
  .where { color: var(--ink-soft); font-size: 0.85rem; }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    margin: 4px 0 6px;
  }
  dt { color: var(--ink-soft); }
  dd { margin: 0; overflow-wrap: anywhere; }
  .hint { color: var(--ink-soft); }

  @media (max-width: 900px) {
    .layout { grid-template-columns: 1fr; }
  }
</style>
