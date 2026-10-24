<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api } from "../lib/api";
  import { canon } from "../lib/canon";
  import ImportCheck from "../components/ImportCheck.svelte";
  import KeyInspector from "../components/KeyInspector.svelte";
  import type {
    Edit,
    HostStatus,
    ImportChoice,
    ImportPreview,
    Imported,
    KeyFields,
    KeyView,
    PluginManifest,
    Pos,
    ProfileSummary,
    ProfileView,
  } from "../lib/types";

  let {
    status: hostStatus,
    onsetup = () => {},
  }: {
    status: HostStatus;
    /** Open Settings at a part of it (the id of its heading or section). */
    onsetup?: (anchor: string) => void;
  } = $props();

  let profiles = $state<ProfileSummary[]>([]);
  let plugins = $state<PluginManifest[]>([]);
  let profileId = $state<string | undefined>(undefined);
  let profile = $state<ProfileView | null>(null);
  let profileMenu = $state(false);
  // A new profile can show Bitfocus Companion's buttons instead of its own.
  let newCompanion = $state(false);
  // Companion's connection state, kept fresh while a Companion profile is open, and the
  // mistake in integrations.toml that keeps it off, if there is one.
  let companionState = $state<string | null>(null);
  let integrationsError = $state<string | null>(null);
  let companionOff = $state(false);
  $effect(() => {
    if (!profile?.companion) return;
    companionState = hostStatus.companion;
    integrationsError = hostStatus.integrationsError ?? null;
    companionOff = (hostStatus.integrationsOff ?? []).includes("Companion");
    const t = setInterval(async () => {
      try {
        const s = await api.status();
        companionState = s.companion;
        integrationsError = s.integrationsError ?? null;
        companionOff = (s.integrationsOff ?? []).includes("Companion");
      } catch {
        // keep the last state
      }
    }, 3000);
    return () => clearInterval(t);
  });
  let profileNaming = $state<null | { mode: "new" } | { mode: "copy" } | { mode: "rename" }>(null);
  let profileNameDraft = $state("");
  let notice = $state<string | null>(null);
  $effect(() => {
    if (notice) {
      const t = setTimeout(() => (notice = null), 3000);
      return () => clearTimeout(t);
    }
  });
  let pageIndex = $state(0);
  let selected = $state<Pos | null>(null);
  let now = $state(new Date());
  let error = $state<string | null>(null);
  let busy = $state(false);

  // The inspector keeps its own draft; it's rebuilt only when the key changes underneath
  // it (undo, another edit, a hand edit), not when its own save comes back.
  let inspectorRev = $state(0);
  let inspectorBasis = "";

  let pageMenu = $state(false);
  let naming = $state<null | { mode: "add" } | { mode: "rename"; from: string }>(null);
  let nameDraft = $state("");
  let dragOver = $state<string | null>(null);

  const page = $derived(profile?.pages[Math.min(pageIndex, profile.pages.length - 1)] ?? null);
  const selectedKey = $derived(
    selected && page ? (page.keys.find((k) => k.row === selected![0] && k.col === selected![1]) ?? null) : null,
  );

  const cells = $derived.by(() => {
    if (!profile || !page) return [];
    const out: { at: Pos; key: KeyView | null }[] = [];
    for (let r = 0; r < profile.rows; r++)
      for (let c = 0; c < profile.cols; c++)
        out.push({ at: [r, c], key: page.keys.find((k) => k.row === r && k.col === c) ?? null });
    return out;
  });

  function resync() {
    const basis = canon(selectedKey?.fields ?? null);
    if (basis !== inspectorBasis) {
      inspectorBasis = basis;
      inspectorRev++;
    }
  }

  /** `sync`: reset the key inspector if the key changed elsewhere. */
  function show(view: ProfileView, sync = true) {
    if (view.id !== profile?.id) {
      pageIndex = 0;
      selected = null;
    }
    profile = view;
    profileId = view.id;
    if (pageIndex >= view.pages.length) pageIndex = view.pages.length - 1;
    if (sync) resync();
  }

  async function load() {
    profiles = await api.profiles();
    if (profileId && !profiles.some((p) => p.id === profileId)) profileId = undefined;
    show(await api.profile(profileId));
  }

  async function openProfile(id: string) {
    profileMenu = false;
    profileId = id;
    try {
      show(await api.profile(id));
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  function startProfileNaming(mode: "new" | "copy" | "rename") {
    profileMenu = false;
    profileNameDraft = mode === "rename" ? (profile?.name ?? "") : mode === "copy" ? `${profile?.name ?? ""} copy` : "";
    profileNaming = { mode };
    newCompanion = false;
    tick().then(() => document.querySelector<HTMLInputElement>("#profile-name")?.select());
  }

  async function submitProfileName() {
    const name = profileNameDraft.trim();
    if (!profileNaming || !name || !profile) {
      profileNaming = null;
      return;
    }
    try {
      if (profileNaming.mode === "rename") {
        show(await api.edit(profile.id, [{ op: "setName", name }]));
      } else {
        const id = await api.createProfile(
          name,
          profileNaming.mode === "copy" ? profile.id : undefined,
          profileNaming.mode === "new" && newCompanion,
        );
        profiles = await api.profiles();
        await openProfile(id);
      }
      profiles = await api.profiles();
      profileNaming = null;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function exportProfile() {
    if (!profile) return;
    profileMenu = false;
    try {
      if (await api.exportProfile(profile.id)) notice = `Exported “${profile.name}”.`;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  // A profile whose keys run things waits here until the person chooses.
  let importing = $state<ImportPreview | null>(null);
  let importBusy = $state(false);
  let profileOptions = $state<HTMLButtonElement | null>(null);

  async function importProfile() {
    profileMenu = false;
    try {
      const s = await api.importProfile();
      if (s.step === "imported") await imported(s);
      else if (s.step === "confirm") importing = s.preview;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function imported(r: Imported) {
    profiles = await api.profiles();
    await openProfile(r.id);
    notice = r.switchedOff
      ? `Profile imported, with ${r.switchedOff} ${r.switchedOff === 1 ? "key" : "keys"} switched off.`
      : "Profile imported.";
  }

  /** The dialog's answer: a choice, or null to cancel. */
  async function chooseImport(choice: ImportChoice | null) {
    const p = importing;
    if (!p || importBusy) return;
    importBusy = true;
    try {
      if (choice) await imported(await api.finishImport(p.token, choice));
      else await api.cancelImport(p.token);
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      importBusy = false;
      importing = null;
      // The menu item that started it is gone; the menu's button is the nearest thing.
      await tick();
      profileOptions?.focus();
    }
  }

  async function deleteProfile() {
    if (!profile) return;
    profileMenu = false;
    try {
      await api.deleteProfile(profile.id);
      profileId = undefined;
      await load();
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  // Edits on their way to the host. While a newer one is pending, an older answer mustn't
  // reset the inspector (that would throw away what was typed since).
  let inFlight = 0;

  async function run(edits: Edit[]): Promise<boolean> {
    if (!profile) return false;
    busy = true;
    inFlight++;
    let counted = true;
    try {
      const view = await api.edit(profile.id, edits);
      inFlight--;
      counted = false;
      show(view, inFlight === 0);
      error = null;
      return true;
    } catch (e) {
      error = String(e);
      return false;
    } finally {
      if (counted) inFlight--;
      busy = inFlight > 0;
    }
  }

  async function history(which: "undo" | "redo") {
    if (!(which === "undo" ? profile?.canUndo : profile?.canRedo)) return;
    try {
      show(await (which === "undo" ? api.undo() : api.redo()));
      error = null;
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  function select(at: Pos | null) {
    selected = at;
    inspectorBasis = canon(
      at && page ? (page.keys.find((k) => k.row === at[0] && k.col === at[1])?.fields ?? null) : null,
    );
    inspectorRev++;
  }

  function goToPage(i: number) {
    pageIndex = i;
    pageMenu = false;
    select(null);
  }

  function save(fields: KeyFields) {
    if (!page || !selected) return;
    inspectorBasis = canon(fields);
    run([{ op: "setKey", page: page.name, at: selected, key: fields }]);
  }

  function clear() {
    if (!page || !selected) return;
    run([{ op: "clearKey", page: page.name, at: selected }]);
  }

  async function move(from: Pos, to: Pos, toPage?: string) {
    if (!page || !profile) return;
    if (to[0] < 0 || to[1] < 0 || to[0] >= profile.rows || to[1] >= profile.cols) return;
    const edit: Edit =
      toPage && toPage !== page.name
        ? { op: "moveKeyToPage", fromPage: page.name, at: from, toPage, to }
        : { op: "moveKey", page: page.name, from, to };
    if (await run([edit])) {
      if (toPage && toPage !== page.name) {
        pageIndex = profile.pages.findIndex((p) => p.name === toPage);
      }
      select(to);
      await tick();
      document.querySelector<HTMLElement>(`[data-cell="${to[0]}-${to[1]}"]`)?.focus();
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

  /** The title the tablet shows: one a Stream Deck plugin set, else the key's own. */
  function shownTitle(k: KeyView): string {
    return k.liveTitle ?? live(k.title);
  }

  /** Stream Deck plugin action id → its name, for keys whose plugin draws no title. */
  let sdActionNames = $state(new Map<string, string>());

  function keyLabel(k: KeyView): string {
    const title = (k.liveTitle || live(k.title)).replaceAll("\n", " ");
    const action = k.fields.action;
    const named = action?.type === "sdPlugin" ? sdActionNames.get(action.action) : undefined;
    return `${title || named || "Untitled"}: ${k.detail}`;
  }

  function keyStyle(k: KeyView): string {
    if (k.kind === "slider") return `color: ${k.fg}`;
    // A plugin's image covers the key; the tablet draws it on black.
    return `background: ${k.liveImage ? "#000" : k.bg}; color: ${k.fg}`;
  }

  // A Stream Deck plugin drew keys. Only the drawings are taken from the answer, so one that
  // crosses an edit on its way can't put a key's old settings back. One refresh at a time;
  // events meanwhile make one more.
  let looksBusy = false;
  let looksAgain = false;
  async function refreshLooks() {
    if (looksBusy) {
      looksAgain = true;
      return;
    }
    looksBusy = true;
    try {
      do {
        looksAgain = false;
        const id = profile?.id;
        if (!id) return;
        const fresh = await api.profile(id);
        if (profile?.id !== id) return;
        for (const p of profile.pages) {
          const drawn = fresh.pages.find((f) => f.name === p.name)?.keys ?? [];
          for (const k of p.keys) {
            const d = drawn.find((f) => f.row === k.row && f.col === k.col);
            k.liveImage = d?.liveImage ?? null;
            k.liveTitle = d?.liveTitle ?? null;
          }
        }
      } while (looksAgain);
    } catch {
      // The next drawing or profile change tries again.
    } finally {
      looksBusy = false;
    }
  }

  function follow(k: KeyView) {
    if (!profile) return;
    const nav = k.fields.nav;
    if (nav && typeof nav === "object" && "profile" in nav) {
      // Show the profile the key switches to (at its page), as the tablet would.
      const page = nav.page;
      openProfile(nav.profile).then(() => {
        const i = page && profile ? profile.pages.findIndex((p) => p.name === page) : -1;
        if (i >= 0) goToPage(i);
      });
    } else if (nav && typeof nav === "object") {
      const i = profile.pages.findIndex((p) => p.name === nav.page);
      if (i >= 0) goToPage(i);
    } else if (nav === "back" || nav === "previous") goToPage(Math.max(0, pageIndex - 1));
    else if (nav === "next") goToPage(Math.min(profile.pages.length - 1, pageIndex + 1));
  }

  function onCellKey(e: KeyboardEvent, at: Pos, k: KeyView | null) {
    const arrows: Record<string, Pos> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    const d = arrows[e.key];
    if (d && e.altKey && k) {
      e.preventDefault();
      move(at, [at[0] + d[0], at[1] + d[1]]);
    } else if (d && profile) {
      e.preventDefault();
      const to: Pos = [Math.max(0, Math.min(profile.rows - 1, at[0] + d[0])), Math.max(0, Math.min(profile.cols - 1, at[1] + d[1]))];
      select(to);
      document.querySelector<HTMLElement>(`[data-cell="${to[0]}-${to[1]}"]`)?.focus();
    } else if ((e.key === "Delete" || e.key === "Backspace") && k) {
      e.preventDefault();
      select(at);
      clear();
    } else if (e.key === "Enter" && k) {
      follow(k);
    }
  }

  async function submitName() {
    const name = nameDraft.trim();
    if (!naming || !name) {
      naming = null;
      return;
    }
    if (naming.mode === "add") {
      if (await run([{ op: "addPage", name }])) {
        goToPage(profile!.pages.length - 1);
        naming = null;
      }
    } else if (await run([{ op: "renamePage", from: naming.from, to: name }])) {
      naming = null;
    }
  }

  function startNaming(mode: "add" | "rename") {
    pageMenu = false;
    nameDraft = mode === "rename" ? (page?.name ?? "") : "";
    naming = mode === "add" ? { mode } : { mode, from: page!.name };
    tick().then(() => document.querySelector<HTMLInputElement>("#page-name")?.select());
  }

  async function movePage(by: number) {
    if (!page || !profile) return;
    const index = pageIndex + by;
    if (index < 0 || index >= profile.pages.length) return;
    pageMenu = false;
    if (await run([{ op: "movePage", name: page.name, index }])) pageIndex = index;
  }

  async function removePage() {
    if (!page) return;
    pageMenu = false;
    const name = page.name;
    if (await run([{ op: "removePage", name }])) goToPage(Math.max(0, pageIndex - 1));
  }

  // Copy and paste keys within the app (the system clipboard keeps text).
  let copied = $state.raw<KeyFields | null>(null);

  function onWindowKey(e: KeyboardEvent) {
    // Keys pressed in the import dialog still reach the window: no undo behind it.
    if (importing) return;
    const typing = (e.target as HTMLElement)?.closest?.("input, textarea, select");
    if (typing || !(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "c" && selectedKey) {
      e.preventDefault();
      copied = $state.snapshot(selectedKey.fields) as KeyFields;
    } else if (k === "v" && copied && selected && page) {
      e.preventDefault();
      run([{ op: "setKey", page: page.name, at: selected, key: copied }]).then(() => select(selected));
    } else if (k === "z" && !e.shiftKey) {
      e.preventDefault();
      history("undo");
    } else if ((k === "z" && e.shiftKey) || k === "y") {
      e.preventDefault();
      history("redo");
    }
  }

  function dragData(e: DragEvent): Pos | null {
    const raw = e.dataTransfer?.getData("application/x-perrito-key");
    if (!raw) return null;
    const [r, c] = raw.split(",").map(Number);
    return [r, c];
  }

  async function openFile() {
    try {
      await api.openProfileFile(profile?.id);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    load();
    api
      .plugins()
      .then((v) => (plugins = v.installed.map((p) => p.manifest)))
      .catch(() => {});
    api
      .sdPlugins()
      .then((v) => (sdActionNames = new Map(v.installed.flatMap((p) => p.actions.map((a) => [a.uuid, a.name] as const)))))
      .catch(() => {});
    const off = api.onHostEvent((e) => {
      if (e.kind === "profileChanged") load();
      else if (e.kind === "keyLooksChanged" && e.profile === profile?.id) refreshLooks();
    });
    const t = setInterval(() => (now = new Date()), 1000);
    return () => {
      clearInterval(t);
      off.then((f) => f());
      // Switching views drops a waiting import rather than leave it in the host.
      if (importing) api.cancelImport(importing.token).catch(() => {});
    };
  });
</script>

<svelte:window onkeydown={onWindowKey} />

<header class="top">
  <div>
    <h1>Buttons</h1>
    {#if profile}
      <div class="profile-bar">
        {#if profileNaming}
          <form class="naming" onsubmit={(e) => (e.preventDefault(), submitProfileName())}>
            <input
              id="profile-name"
              aria-label={profileNaming.mode === "rename" ? "Profile name" : "New profile name"}
              bind:value={profileNameDraft}
              onkeydown={(e) => e.key === "Escape" && (profileNaming = null)}
              maxlength="60"
            />
            <button class="btn small primary" type="submit"
              >{profileNaming.mode === "rename" ? "Rename" : profileNaming.mode === "copy" ? "Make copy" : "Create"}</button
            >
            <button class="btn small quiet" type="button" onclick={() => (profileNaming = null)}>Cancel</button>
            {#if profileNaming.mode === "new"}
              <label class="check">
                <input type="checkbox" bind:checked={newCompanion} />
                Show Bitfocus Companion’s buttons
              </label>
            {/if}
          </form>
        {:else}
          <label class="sr-only" for="profile-select">Profile</label>
          <select id="profile-select" value={profile.id} onchange={(e) => openProfile(e.currentTarget.value)}>
            {#each profiles as p (p.id)}
              <option value={p.id}>{p.name}{p.isDefault ? " (default)" : ""}{p.hasError ? " — has a mistake" : ""}</option>
            {/each}
          </select>
          <span class="sub">{profile.companion ? "From Companion" : `${profile.cols} × ${profile.rows} keys`}</span>
          <button class="btn small quiet" onclick={() => startProfileNaming("new")}>+ Profile</button>
          <div class="menu-wrap">
            <button
              class="btn small quiet"
              bind:this={profileOptions}
              aria-expanded={profileMenu}
              aria-haspopup="menu"
              onclick={() => (profileMenu = !profileMenu)}>Profile options</button
            >
            {#if profileMenu}
              <div class="menu" role="menu">
                <button role="menuitem" onclick={() => startProfileNaming("rename")}>Rename “{profile.name}”</button>
                <button role="menuitem" onclick={() => startProfileNaming("copy")}>Make a copy</button>
                <button role="menuitem" onclick={openFile}>Open the file</button>
                <button role="menuitem" onclick={exportProfile}>Export…</button>
                <button role="menuitem" onclick={importProfile}>Import a profile…</button>
                <button
                  role="menuitem"
                  class="danger"
                  onclick={deleteProfile}
                  disabled={profiles.find((p) => p.id === profile?.id)?.isDefault}>Delete profile</button
                >
              </div>
            {/if}
          </div>
        {/if}
      </div>
    {/if}
  </div>
  <div class="actions">
    <button class="btn quiet" onclick={() => history("undo")} disabled={!profile?.canUndo} title="Undo (Ctrl+Z)">Undo</button>
    <button class="btn quiet" onclick={() => history("redo")} disabled={!profile?.canRedo} title="Redo (Ctrl+Shift+Z)">Redo</button>
  </div>
</header>

{#if profile?.reloadError}
  <div class="banner" role="alert">
    <strong>Your last edit to the file wasn’t applied.</strong> The tablets still show the version before it. Fix this and
    save again:
    <pre class="selectable">{profile.reloadError}</pre>
  </div>
{/if}

{#if notice}
  <p class="notice" role="status">{notice}</p>
{/if}

{#if error}
  <div class="banner" role="alert">
    <span class="selectable">{error}</span>
    <button class="link" onclick={() => (error = null)}>Dismiss</button>
  </div>
{/if}

{#if profile?.companion}
  <section class="companion" aria-labelledby="companion-h">
    <h2 id="companion-h">Buttons from Bitfocus Companion</h2>
    <p>
      Tablets showing “{profile.name}” become surfaces in Companion: set up their buttons there, and presses on the
      tablet go straight to Companion. Each tablet keeps its own grid size.
    </p>
    {#if companionState !== null}
      <p class="state" role="status">{companionState}</p>
    {:else if integrationsError}
      <!-- The [companion] lines may well be there already; the mistake is what's in the way. -->
      <p class="state mistake" role="alert">integrations.toml has a mistake: {integrationsError}</p>
      <p>PerritoDeck ignores the whole file until it’s fixed, so Companion stays off.</p>
      <span><button class="btn" onclick={() => onsetup("integrations")}>Open Settings → Integrations</button></span>
    {:else if companionOff}
      <p>Companion is turned off in Settings → Integrations.</p>
      <span><button class="btn" onclick={() => onsetup("integrations-companion")}>Open Settings → Integrations</button></span>
    {:else}
      <p>First tell PerritoDeck where Companion runs.</p>
      <span><button class="btn" onclick={() => onsetup("integrations-companion")}>Set up Companion</button></span>
    {/if}
  </section>
{:else if profile && page}
  <div class="pages">
    <div class="tabs" role="tablist" aria-label="Pages">
      {#each profile.pages as p, i (p.name)}
        <button
          role="tab"
          aria-selected={i === pageIndex}
          class:active={i === pageIndex}
          class:drop={dragOver === `tab-${i}`}
          onclick={() => goToPage(i)}
          ondblclick={() => {
            goToPage(i);
            startNaming("rename");
          }}
          ondragover={(e) => {
            if (i !== pageIndex) {
              e.preventDefault();
              dragOver = `tab-${i}`;
            }
          }}
          ondragleave={() => (dragOver = null)}
          ondrop={(e) => {
            e.preventDefault();
            dragOver = null;
            const from = dragData(e);
            if (from) move(from, from, p.name);
          }}>{p.name}</button
        >
      {/each}
    </div>
    {#if naming}
      <form class="naming" onsubmit={(e) => (e.preventDefault(), submitName())}>
        <input
          id="page-name"
          aria-label={naming.mode === "add" ? "New page name" : "Page name"}
          bind:value={nameDraft}
          onkeydown={(e) => e.key === "Escape" && (naming = null)}
          maxlength="40"
        />
        <button class="btn small primary" type="submit">{naming.mode === "add" ? "Add page" : "Rename"}</button>
        <button class="btn small quiet" type="button" onclick={() => (naming = null)}>Cancel</button>
      </form>
    {:else}
      <button class="btn small quiet" onclick={() => startNaming("add")}>+ Page</button>
      <div class="menu-wrap">
        <button class="btn small quiet" aria-expanded={pageMenu} aria-haspopup="menu" onclick={() => (pageMenu = !pageMenu)}
          >Page options</button
        >
        {#if pageMenu}
          <div class="menu" role="menu">
            <button role="menuitem" onclick={() => startNaming("rename")}>Rename “{page.name}”</button>
            <button role="menuitem" onclick={() => movePage(-1)} disabled={pageIndex === 0}>Move left</button>
            <button role="menuitem" onclick={() => movePage(1)} disabled={pageIndex === profile.pages.length - 1}
              >Move right</button
            >
            <button role="menuitem" class="danger" onclick={removePage} disabled={profile.pages.length < 2}>Remove page</button>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <div class="layout">
    <div class="slab" class:busy style="--cols: {profile.cols}; --rows: {profile.rows}" role="grid" aria-label="Keys on {page.name}">
      {#each cells as { at, key: k } (`${page.name}-${at[0]}-${at[1]}`)}
        {@const isSel = selected?.[0] === at[0] && selected?.[1] === at[1]}
        <button
          class="key"
          class:empty={!k}
          class:selected={isSel}
          class:drop={dragOver === `${at[0]}-${at[1]}`}
          data-cell="{at[0]}-{at[1]}"
          class:slider={k?.kind === "slider"}
          class:drawn={!!k?.liveImage && k.kind !== "slider"}
          style={k ? keyStyle(k) : ""}
          aria-label={k ? keyLabel(k) : `Empty, row ${at[0] + 1} column ${at[1] + 1}`}
          aria-pressed={isSel}
          draggable={!!k}
          onclick={() => select(at)}
          ondblclick={() => k && follow(k)}
          onkeydown={(e) => onCellKey(e, at, k)}
          ondragstart={(e) => {
            e.dataTransfer?.setData("application/x-perrito-key", `${at[0]},${at[1]}`);
            if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
          }}
          ondragover={(e) => {
            e.preventDefault();
            dragOver = `${at[0]}-${at[1]}`;
          }}
          ondragleave={() => (dragOver = null)}
          ondrop={(e) => {
            e.preventDefault();
            dragOver = null;
            const from = dragData(e);
            if (from && (from[0] !== at[0] || from[1] !== at[1])) move(from, at);
          }}
        >
          {#if k?.kind === "slider"}
            <span class="slider-fill" style="background: {k.bg}"></span>
            <span class="slider-title">{live(k.title)}</span>
          {:else if k?.liveImage}
            <!-- What its Stream Deck plugin drew, filling the key with the title over it, as on the tablet. -->
            <img class="live" src={k.liveImage} alt="" />
            {#if shownTitle(k)}<span class="live-title">{shownTitle(k)}</span>{/if}
          {:else if k}
            {#if k.iconData}<img class="icon" class:with-title={!!shownTitle(k)} src={k.iconData} alt="" />{/if}
            {#if shownTitle(k)}<span>{shownTitle(k)}</span>{/if}
          {:else}<span class="plus" aria-hidden="true">+</span>{/if}
        </button>
      {/each}
    </div>

    <aside class="side">
      {#if selected}
        {#key `${page.name}:${selected[0]}:${selected[1]}:${inspectorRev}`}
          <KeyInspector
            key={selectedKey}
            at={selected}
            page={page.name}
            profileId={profile.id}
            pageNames={profile.pages.map((p) => p.name)}
            {profiles}
            onsave={save}
            onclear={clear}
            {plugins}
          />
        {/key}
      {:else}
        <p class="hint">Select a key to change it, or an empty spot to add one.</p>
        <p class="hint">Drag keys to move them, or onto a page tab to send them there.</p>
        <p class="hint">With a key selected: arrows move the selection, Alt+arrows move the key, Delete removes it, Enter follows a page key, Ctrl+C and Ctrl+V copy it to another spot.</p>
        <p class="hint">Changes save to the profile file and reach the tablets straight away.</p>
      {/if}
    </aside>
  </div>
{/if}

{#if importing}
  <ImportCheck preview={importing} busy={importBusy} onchoose={chooseImport} />
{/if}

<style>
  .top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    margin-bottom: 18px;
  }
  .sub { color: var(--ink-soft); font-size: 0.9rem; }
  .profile-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }
  .profile-bar select {
    font: 700 0.95rem var(--body);
    color: var(--ink);
    background: var(--paper);
    border: 1.5px solid var(--line);
    border-radius: 999px;
    padding: 5px 12px;
    max-width: 260px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
  .actions { display: flex; gap: 4px; align-items: center; }
  .actions .btn:disabled { opacity: 0.4; cursor: default; }

  .banner {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    align-items: baseline;
    margin-bottom: 16px;
    padding: 12px 16px;
    border-radius: 12px;
    border: 1.5px solid var(--danger);
    background: var(--paper);
    max-width: 980px;
    white-space: pre-line;
  }
  .banner pre {
    flex-basis: 100%;
    margin: 4px 0 0;
    white-space: pre-wrap;
    font-size: 0.85rem;
  }
  .notice { color: var(--ink-soft); margin: -8px 0 12px; }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--apricot-deep);
    text-decoration: underline;
  }

  .pages {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-bottom: 14px;
  }
  .tabs { display: flex; flex-wrap: wrap; gap: 6px; }
  .tabs button {
    padding: 5px 14px;
    border-radius: 999px;
    border: 1.5px solid var(--line);
    background: none;
    color: var(--ink-soft);
    font-weight: 700;
  }
  .tabs button.active { background: var(--ink); border-color: var(--ink); color: var(--fog); }
  .tabs button.drop { border-color: var(--apricot); border-style: dashed; color: var(--ink); }
  .btn.small { padding: 5px 12px; font-size: 0.85rem; }
  .naming { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .naming .check { display: flex; align-items: center; gap: 6px; font-size: 0.9rem; color: var(--ink-soft); }
  .companion { max-width: 64ch; display: grid; gap: 12px; margin-top: 8px; }
  .companion .state { font-weight: 700; }
  .companion .mistake { color: var(--danger); }
  .naming input {
    font: inherit;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1.5px solid var(--apricot);
    background: var(--paper);
    color: var(--ink);
    width: 12em;
  }
  .menu-wrap { position: relative; }
  .menu {
    position: absolute;
    z-index: 5;
    top: calc(100% + 6px);
    left: 0;
    display: grid;
    min-width: 190px;
    padding: 6px;
    border-radius: 12px;
    background: var(--paper);
    border: 1px solid var(--line);
    box-shadow: 0 12px 30px -12px rgb(0 0 0 / 0.35);
  }
  .menu button {
    text-align: left;
    padding: 7px 10px;
    border: 0;
    border-radius: 8px;
    background: none;
  }
  .menu button:hover:not(:disabled) { background: var(--fog); }
  .menu button:disabled { opacity: 0.4; cursor: default; }
  .menu .danger { color: var(--danger); }

  .layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    gap: 28px;
    align-items: start;
  }

  /* The tablet, drawn as it looks: a dark slab with rounded keys. */
  .slab {
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    gap: clamp(6px, 1.1vw, 12px);
    padding: clamp(12px, 1.8vw, 22px);
    border-radius: 26px;
    background: var(--deck);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--deck) 70%, #000), 0 18px 40px -18px rgb(0 0 0 / 0.45);
    max-width: 760px;
  }
  .slab.busy { cursor: progress; }

  .key {
    aspect-ratio: 1;
    border: 0;
    border-radius: 14%;
    display: grid;
    place-items: center;
    padding: 6%;
    font: 700 clamp(0.6rem, 1.05vw, 0.95rem)/1.15 var(--display);
    text-align: center;
    white-space: pre-line;
    overflow: hidden;
    transition: transform 80ms ease-out;
  }
  .key:not(.empty):hover { transform: translateY(-1px); }
  .key:active { transform: scale(0.96); }
  .key.selected { outline: 3px solid var(--apricot); outline-offset: 3px; }
  .key.empty {
    background: color-mix(in srgb, var(--deck) 82%, #fff);
    color: transparent;
    opacity: 0.45;
  }
  .key.empty:hover,
  .key.empty.selected,
  .key.empty:focus-visible { opacity: 0.8; color: #fff; }
  .key.drop { outline: 3px dashed var(--apricot); outline-offset: 2px; }
  .plus { font: 400 1.6rem/1 var(--display); }
  .key.slider {
    position: relative;
    background: color-mix(in srgb, var(--deck) 70%, #fff);
    place-items: start center;
  }
  .slider-fill {
    position: absolute;
    inset: 50% 0 0 0;
  }
  .slider-title { position: relative; padding-top: 6%; }
  .key:has(.icon) { place-items: center; align-content: center; gap: 4%; }
  .icon { width: 62%; height: 62%; object-fit: contain; pointer-events: none; }
  .icon.with-title { width: 48%; height: 48%; }
  /* A plugin's drawing: the image fills the key and the title sits on its lower part,
     outlined so it reads over any picture. */
  .key.drawn { position: relative; }
  .live {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    pointer-events: none;
  }
  /* At the bottom, as on the tablet, which shrinks a title until it fits there: a longer
     one grows upward here instead of being cut off at the key's edge. */
  .live-title {
    position: absolute;
    left: 6%;
    right: 6%;
    bottom: 6%;
    min-height: 39%;
    max-height: 88%;
    display: grid;
    place-items: center;
    font-size: 0.85em;
    line-height: 1.1;
    overflow: hidden;
    text-shadow: -1px 0 #000, 1px 0 #000, 0 -1px #000, 0 1px #000;
  }

  .side { display: grid; gap: 10px; }
  .hint { color: var(--ink-soft); font-size: 0.92rem; }

  @media (max-width: 980px) {
    .layout { grid-template-columns: 1fr; }
  }
</style>
