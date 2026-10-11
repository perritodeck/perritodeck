<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { OnlineEntry, PluginField, PluginStatus, PluginsView, SdPlugin, SdPluginsView } from "../lib/types";

  let view = $state<PluginsView | null>(null);
  let error = $state<string | null>(null);
  let confirming = $state<string | null>(null);
  // The online catalogue is fetched only when asked for.
  let online = $state<OnlineEntry[] | null>(null);
  let looking = $state(false);
  let onlineError = $state<string | null>(null);
  let installing = $state<string | null>(null);

  async function lookOnline() {
    looking = true;
    onlineError = null;
    try {
      online = await api.onlinePlugins();
    } catch (e) {
      onlineError = String(e);
    } finally {
      looking = false;
    }
  }

  async function installOnline(id: string) {
    installing = id;
    try {
      await act(() => api.installOnlinePlugin(id));
      if (online) online = await api.onlinePlugins().catch(() => online);
    } finally {
      installing = null;
    }
  }

  let sd = $state<SdPluginsView | null>(null);
  // The plugin whose settings are open, and the draft.
  let configuring = $state<string | null>(null);
  let draft = $state<Record<string, unknown>>({});
  let savedNote = $state<string | null>(null);

  async function configure(id: string) {
    if (configuring === id) {
      configuring = null;
      return;
    }
    try {
      draft = await api.pluginSettings(id);
      configuring = id;
      savedNote = null;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function saveSettings(id: string, fields: PluginField[]) {
    const out: Record<string, unknown> = {};
    for (const f of fields) {
      const v = draft[f.id];
      if (v === undefined || v === "") continue;
      out[f.id] = f.type === "number" && typeof v === "string" && !isNaN(Number(v)) ? Number(v) : v;
    }
    try {
      await api.setPluginSettings(id, out);
      savedNote = "Saved. The plugin uses the new settings now.";
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function load() {
    view = await api.plugins();
    sd = await api.sdPlugins().catch(() => null);
  }

  function sdStatus(s: SdPlugin["status"]): string {
    switch (s.state) {
      case "running":
        return "Running";
      case "starting":
        return "Starting…";
      case "needsNode":
        return "Needs Node.js, which isn't installed";
      case "unsupported":
        return `Can't run here: ${s.reason}`;
      case "crashed":
        return `Stopped working: ${s.reason}`;
    }
  }

  async function act(f: () => Promise<unknown>) {
    try {
      await f();
      error = null;
    } catch (e) {
      error = String(e);
    }
    await load();
  }

  function statusText(s: PluginStatus): string {
    switch (s.state) {
      case "running":
        return "Running";
      case "starting":
        return "Starting…";
      case "missingRuntime":
        return `Needs ${s.runtime}, which isn't installed`;
      case "crashed":
        return `Stopped working: ${s.reason}`;
      case "stopped":
        return "Stopped";
    }
  }

  onMount(() => {
    load();
    // Plugins start and stop in the background; keep the list current.
    const t = setInterval(load, 3000);
    return () => clearInterval(t);
  });
</script>

<header>
  <div>
    <h1>Plugins</h1>
    <p class="intro">Plugins add things keys can do, and values titles can show.</p>
  </div>
  <div class="actions">
    <button class="btn quiet" onclick={() => act(api.openPluginsFolder)}>Open plugins folder</button>
    <button class="btn" onclick={() => act(api.installPluginFolder)}>Install from folder…</button>
  </div>
</header>

{#if error}
  <p class="err" role="alert">{error}</p>
{/if}

{#if view}
  <section aria-labelledby="installed">
    <h2 id="installed">Installed</h2>
    {#if view.installed.length === 0}
      <p class="empty">None yet. Add one below, or install one you’ve downloaded.</p>
    {:else}
      <ul>
        {#each view.installed as p (p.manifest.id)}
          <li>
            <span class="dot" class:on={p.status.state === "running"} class:bad={p.status.state === "crashed" || p.status.state === "missingRuntime"} aria-hidden="true"></span>
            <div class="what">
              <strong>{p.manifest.name}</strong> <span class="version">{p.manifest.version}</span>
              <span class="desc">{p.manifest.description}</span>
              <span class="status" class:bad={p.status.state === "crashed" || p.status.state === "missingRuntime"}
                >{statusText(p.status)}</span
              >
            </div>
            {#if confirming === p.manifest.id}
              <div class="confirm" role="group" aria-label="Remove {p.manifest.name}?">
                <span>Keys using it will stop working.</span>
                <button class="btn danger small" onclick={() => act(() => api.removePlugin(p.manifest.id)).then(() => (confirming = null))}
                  >Remove</button
                >
                <button class="btn quiet small" onclick={() => (confirming = null)}>Keep</button>
              </div>
            {:else}
              {#if p.manifest.settings?.length}
                <button
                  class="btn small"
                  class:quiet={configuring !== p.manifest.id}
                  aria-expanded={configuring === p.manifest.id}
                  onclick={() => configure(p.manifest.id)}
                  aria-label="Settings for {p.manifest.name}">Settings</button
                >
              {/if}
              <button class="btn quiet small" onclick={() => act(() => api.restartPlugin(p.manifest.id))}>Restart</button>
              <button class="btn quiet small" onclick={() => (confirming = p.manifest.id)}>Remove…</button>
            {/if}
          </li>
          {#if configuring === p.manifest.id && p.manifest.settings}
            {@const fields = p.manifest.settings}
            <li class="settings">
              <form class="form" onsubmit={(e) => (e.preventDefault(), saveSettings(p.manifest.id, fields))}>
                {#each fields as f (f.id)}
                  <div class="field">
                    {#if f.type === "checkbox"}
                      <label class="check">
                        <input type="checkbox" checked={!!(draft[f.id] ?? f.default)} onchange={(e) => (draft[f.id] = e.currentTarget.checked)} />
                        {f.label}
                      </label>
                    {:else if f.type === "select"}
                      <label for="ps-{f.id}">{f.label}</label>
                      <select id="ps-{f.id}" value={String(draft[f.id] ?? f.default ?? f.options?.[0]?.value ?? "")} onchange={(e) => (draft[f.id] = e.currentTarget.value)}>
                        {#each f.options ?? [] as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
                      </select>
                    {:else}
                      <label for="ps-{f.id}">{f.label}</label>
                      <input
                        id="ps-{f.id}"
                        type={f.type === "password" ? "password" : "text"}
                        inputmode={f.type === "number" ? "decimal" : undefined}
                        autocomplete="off"
                        spellcheck="false"
                        placeholder={f.placeholder ?? ""}
                        value={String(draft[f.id] ?? f.default ?? "")}
                        oninput={(e) => (draft[f.id] = e.currentTarget.value)}
                      />
                    {/if}
                  </div>
                {/each}
                <div class="save-row">
                  <button class="btn small primary" type="submit">Save settings</button>
                  {#if savedNote}<span role="status">{savedNote}</span>{/if}
                </div>
                <small>Kept on this computer only, in the settings folder, readable by your user account.</small>
              </form>
            </li>
          {/if}
        {/each}
      </ul>
    {/if}
    {#each view.broken as b (b.folder)}
      <p class="err">Can’t load <code>{b.folder}</code>: {b.problem}</p>
    {/each}
  </section>

  {#if sd}
    <section aria-labelledby="sd">
      <div class="section-head">
        <h2 id="sd">Stream Deck plugins</h2>
        <span class="actions">
          <button class="btn small" onclick={() => act(() => api.installSdPlugin(false))}>Add a plugin file…</button>
          <button class="btn small quiet" onclick={() => act(() => api.installSdPlugin(true))}>From a folder…</button>
        </span>
      </div>
      {#if sd.installed.length === 0}
        <p class="empty">
          Many plugins made for the Elgato Stream Deck also work here: add a <code>.streamDeckPlugin</code> file. Plugins
          written in JavaScript need Node.js.
        </p>
      {:else}
        <ul>
          {#each sd.installed as p (p.uuid)}
            {@const bad = p.status.state !== "running" && p.status.state !== "starting"}
            <li>
              {#if p.icon}<img class="sd-icon" src={p.icon} alt="" />{:else}<span class="dot" class:on={p.status.state === "running"} class:bad aria-hidden="true"></span>{/if}
              <div class="what">
                <strong>{p.name}</strong> <span class="version">{p.version}</span>
                <span class="desc">{p.description || p.actions.map((a) => a.name).join(", ")}</span>
                <span class="status" class:bad>{sdStatus(p.status)}</span>
              </div>
              {#if confirming === p.uuid}
                <div class="confirm" role="group" aria-label="Remove {p.name}?">
                  <span>Keys using it will stop working.</span>
                  <button class="btn danger small" onclick={() => act(() => api.removeSdPlugin(p.uuid)).then(() => (confirming = null))}
                    >Remove</button
                  >
                  <button class="btn quiet small" onclick={() => (confirming = null)}>Keep</button>
                </div>
              {:else}
                <button class="btn quiet small" onclick={() => act(() => api.restartSdPlugin(p.uuid))} aria-label="Restart {p.name}">Restart</button>
                <button class="btn quiet small" onclick={() => (confirming = p.uuid)} aria-label="Remove {p.name}…">Remove…</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      {#each sd.broken as b (b.folder)}
        <p class="err">Can’t load <code>{b.folder}</code>: {b.problem}</p>
      {/each}
    </section>
  {/if}

  {#if view.catalog.length}
    <section aria-labelledby="catalog">
      <h2 id="catalog">Comes with PerritoDeck</h2>
      <ul>
        {#each view.catalog as c (c.manifest.id)}
          <li>
            <div class="what">
              <strong>{c.manifest.name}</strong>
              <span class="desc">{c.manifest.description}</span>
            </div>
            {#if c.installed}
              <span class="added">Added</span>
            {:else}
              <button class="btn small" onclick={() => act(() => api.installCatalogPlugin(c.manifest.id))}>Add</button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section aria-labelledby="online">
    <h2 id="online">More plugins online</h2>
    {#if online === null}
      <p class="empty">
        PerritoDeck doesn’t look online until you ask.
        <button class="btn small" onclick={lookOnline} disabled={looking}>{looking ? "Looking…" : "Look online"}</button>
      </p>
    {:else if online.length === 0}
      <p class="empty">The catalogue is empty for now.</p>
    {:else}
      <ul>
        {#each online as o (o.id)}
          <li>
            <div class="what">
              <strong>{o.name}</strong> <span class="version">{o.version}</span>
              <span class="desc">{o.description}</span>
              {#if o.author}<span class="status">By {o.author}</span>{/if}
            </div>
            {#if installing === o.id}
              <span class="added" role="status">Downloading…</span>
            {:else if o.update}
              <button class="btn small" onclick={() => installOnline(o.id)}>Update from {o.installed}</button>
            {:else if o.installed}
              <span class="added">Added</span>
            {:else}
              <button class="btn small" onclick={() => installOnline(o.id)} aria-label="Add {o.name}">Add</button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if onlineError}
      <p class="err" role="alert">{onlineError}</p>
    {/if}
  </section>

  <p class="hint">
    Plugins are programs that run on this computer with your permissions, so only install ones you trust. To write
    one, see <code>plugins/</code> in the PerritoDeck source (SDKs for Python and Node.js).
  </p>
{/if}

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    max-width: 860px;
    margin-bottom: 18px;
  }
  .intro { color: var(--ink-soft); margin-top: 6px; }
  .actions { display: flex; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }
  section { max-width: 860px; margin-bottom: 26px; }
  h2 { margin-bottom: 8px; }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 4px;
    border-bottom: 1px solid var(--line);
  }
  .what { display: grid; flex: 1; gap: 2px; }
  .version { color: var(--ink-soft); font-size: 0.85rem; }
  .desc { color: var(--ink-soft); font-size: 0.9rem; }
  .status { font-size: 0.85rem; color: var(--ink-soft); }
  .status.bad { color: var(--danger); }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--ink-soft);
  }
  .dot.on { background: var(--apricot); border-color: var(--apricot); }
  .dot.bad { border-color: var(--danger); background: var(--danger); }
  .added { color: var(--ink-soft); font-size: 0.9rem; }
  .confirm { display: flex; align-items: center; gap: 6px; }
  .confirm span { color: var(--ink-soft); font-size: 0.9rem; }
  .btn.small { padding: 5px 12px; font-size: 0.85rem; }
  .empty, .hint { color: var(--ink-soft); }
  .empty .btn { margin-left: 8px; }
  .section-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 8px; }
  .section-head h2 { margin: 0; }
  li.settings { display: block; padding: 4px 4px 16px 26px; }
  li.settings .form { max-width: 520px; display: grid; gap: 10px; }
  .save-row { display: flex; gap: 10px; align-items: center; }
  .check { display: flex; align-items: center; gap: 8px; }
  .sd-icon { width: 28px; height: 28px; border-radius: 6px; flex: none; }
  .hint { max-width: 70ch; font-size: 0.9rem; }
  .err { color: var(--danger); margin-bottom: 12px; }
</style>
