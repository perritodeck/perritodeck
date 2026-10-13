<script lang="ts">
  import LiveValuePicker from "./LiveValuePicker.svelte";
  import { untrack } from "svelte";
  import { DEFAULT_BG, DEFAULT_FG } from "../lib/describe";
  import { api } from "../lib/api";
  import { iconPngBase64, iconSvg } from "../lib/icons";
  import ActionPicker, { type Picked } from "./ActionPicker.svelte";
  import IconPicker from "./IconPicker.svelte";
  import type { KeyFields, KeyView, PluginManifest, Pos, ProfileSummary, StateFields } from "../lib/types";

  let {
    key,
    at,
    page,
    pageNames,
    onsave,
    onclear,
    plugins = [],
    profileId = "",
    profiles = [],
  }: {
    key: KeyView | null;
    at: Pos;
    page: string;
    /** The profile the key belongs to (for Stream Deck plugin settings pages). */
    profileId?: string;
    pageNames: string[];
    onsave: (fields: KeyFields) => void;
    onclear: () => void;
    /** Installed plugins, whose actions keys can run. */
    plugins?: PluginManifest[];
    /** Every profile, for keys that switch to one. */
    profiles?: ProfileSummary[];
  } = $props();

  // The draft starts from the key once; the inspector is re-created when the selection
  // (or the key underneath it) changes from outside.
  const initial = untrack(() => ({ key, page, pageNames, plugins, profiles }));
  // A plain copy: the key comes from the page's reactive state.
  const f: KeyFields = (initial.key ? ($state.snapshot(initial.key.fields) as KeyFields) : null) ?? {
    title: null,
    bg: null,
    fg: null,
    action: null,
    nav: null,
    icon: null,
    slider: null,
    states: [],
    follow: null,
  };

  let title = $state(f.title ?? "");
  let icon = $state(f.icon);
  let iconPreview = $state<string | null>(initial.key?.iconData ?? null);
  let bg = $state(f.bg);
  let fg = $state(f.fg);
  let kind = $state<"key" | "toggle">(f.states.length >= 2 ? "toggle" : "key");
  let picked = $state.raw<Picked | string>({ action: f.action, nav: f.nav, slider: f.slider });

  // Toggle states: what each looks like and does. Pickers for each state report here.
  type Draft = { title: string; bg: string | null; fg: string | null; picked: Picked | string };
  const toDraft = (s: StateFields): Draft => ({
    title: s.title ?? "",
    bg: s.bg ?? null,
    fg: s.fg ?? null,
    picked: { action: s.action ?? null, nav: null, slider: null },
  });
  let states = $state<Draft[]>(
    f.states.length >= 2
      ? f.states.map(toDraft)
      : [
          { title: f.title ?? "", bg: null, fg: null, picked: { action: f.action, nav: null, slider: null } },
          { title: "", bg: "#F38BA8", fg: "#11111B", picked: { action: f.action, nav: null, slider: null } },
        ],
  );
  let stateIndex = $state(0);
  let follow = $state(f.follow ?? "");
  // A fresh picker per state so each keeps its own draft.
  let stateRev = $state(0);

  let problem = $state<string | null>(null);
  let choosingIcon = $state(false);

  /** Build the key from the draft, or explain what's missing. */
  function build(): KeyFields | string {
    const common = { title: title === "" ? null : title, bg, fg, icon };
    if (kind === "key") {
      if (typeof picked === "string") return picked;
      return { ...common, ...picked, states: [], follow: null };
    }
    const out: StateFields[] = [];
    for (const [i, s] of states.entries()) {
      if (typeof s.picked === "string") return `State ${i + 1}: ${s.picked}`;
      out.push({
        ...(s.title !== "" ? { title: s.title } : {}),
        ...(s.bg ? { bg: s.bg } : {}),
        ...(s.fg ? { fg: s.fg } : {}),
        ...(s.picked.action ? { action: s.picked.action } : {}),
      });
    }
    if (follow.trim() && out.length !== 2) return "A toggle that follows a value needs exactly two states.";
    return { ...common, action: null, nav: null, slider: null, states: out, follow: follow.trim() || null };
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Save shortly after the last change, so typing makes one edit rather than many. */
  function changed(delay = 350) {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const out = build();
      if (typeof out === "string") {
        // The action picker shows its own problems.
        problem = kind === "key" && typeof picked === "string" ? null : out;
        return;
      }
      problem = null;
      // Plain data, not the draft's reactive objects.
      onsave($state.snapshot(out) as KeyFields);
    }, delay);
  }

  async function pickBuiltin(name: string) {
    choosingIcon = false;
    try {
      const color = (kind === "toggle" ? states[0].fg : null) ?? fg ?? DEFAULT_FG;
      icon = await api.importIconData(await iconPngBase64(name, color));
      iconPreview = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(iconSvg(name, color, 96));
      changed(0);
    } catch (e) {
      problem = String(e);
    }
  }

  async function pickFile() {
    choosingIcon = false;
    try {
      const chosen = await api.chooseIcon();
      if (chosen) {
        icon = chosen;
        iconPreview = null;
        changed(0);
      }
    } catch (e) {
      problem = String(e);
    }
  }

  const where = $derived(`Row ${at[0] + 1}, column ${at[1] + 1}`);
  const colorFields = [
    { label: "Background", get: () => bg, set: (v: string | null) => (bg = v), def: DEFAULT_BG },
    { label: "Text", get: () => fg, set: (v: string | null) => (fg = v), def: DEFAULT_FG },
  ];
</script>

<form class="inspector form" onsubmit={(e) => e.preventDefault()} aria-label="Key settings">
  <header>
    <p class="where">{where} on {page}</p>
    {#if key}
      <button type="button" class="btn quiet danger small" onclick={onclear}>Remove key</button>
    {/if}
  </header>

  {#if key?.fields.action?.type === "sdPlugin"}
    <!-- Plugins draw only keys a tablet shows; the preview has the key's own look until then. -->
    <p class="note">
      The preview shows what its Stream Deck plugin draws while a tablet shows this page. A title or image the plugin
      draws takes the place of the one set here.
    </p>
  {/if}

  <div class="kind" role="radiogroup" aria-label="Kind of key">
    <label><input type="radio" name="kind" value="key" bind:group={kind} onchange={() => changed(0)} /> Key</label>
    <label
      ><input type="radio" name="kind" value="toggle" bind:group={kind} onchange={() => changed(0)} /> Toggle (switches between
      looks)</label
    >
  </div>

  <div class="field">
    <label for="f1">{kind === "toggle" ? "Title (when a state has none)" : "Title"}</label>
    <textarea id="f1" rows="2" bind:value={title} oninput={() => changed()} placeholder={"Two lines\nat most"}></textarea>
    <small>Live values: <code>{"{time}"}</code>, <code>{"{date}"}</code>, <code>{"{cpu}"}</code>, <code>{"{ha:sensor.x}"}</code>…</small>
    <LiveValuePicker
      plugins={initial.plugins}
      onpick={(name) => {
        title = `${title}${title && !title.endsWith("\n") && !title.endsWith(" ") ? " " : ""}{${name}}`;
        changed(0);
      }}
    />
  </div>

  <div class="field">
    <span id="lbl-image">Image</span>
    <div class="image-row">
      {#if iconPreview || icon}
        <span class="thumb" style="background: {bg ?? DEFAULT_BG}">
          {#if iconPreview}<img src={iconPreview} alt="" />{/if}
        </span>
      {/if}
      <button type="button" class="btn small" aria-describedby="lbl-image" aria-expanded={choosingIcon} onclick={() => (choosingIcon = !choosingIcon)}
        >{icon ? "Change…" : "Choose…"}</button
      >
      {#if icon}
        <button
          type="button"
          class="link"
          onclick={() => {
            icon = null;
            iconPreview = null;
            changed(0);
          }}>Remove</button
        >
      {/if}
    </div>
    {#if choosingIcon}
      <IconPicker color={fg ?? DEFAULT_FG} onpick={pickBuiltin} onfile={pickFile} onclose={() => (choosingIcon = false)} />
    {/if}
  </div>

  <div class="colors">
    {#each colorFields as c (c.label)}
      <div class="field">
        <span id="lbl-{c.label}">{c.label}</span>
        <div class="swatch-row">
          <input
            type="color"
            aria-labelledby="lbl-{c.label}"
            value={(c.get() ?? c.def).toLowerCase()}
            oninput={(e) => {
              c.set(e.currentTarget.value.toUpperCase());
              changed(150);
            }}
          />
          <code>{c.get() ?? "default"}</code>
          {#if c.get()}
            <button
              type="button"
              class="link"
              onclick={() => {
                c.set(null);
                changed(0);
              }}>Reset</button
            >
          {/if}
        </div>
      </div>
    {/each}
  </div>

  {#if kind === "key"}
    <ActionPicker
      value={typeof picked === "string" ? { action: null, nav: null, slider: null } : picked}
      {page}
      pageNames={initial.pageNames}
      plugins={initial.plugins}
      profiles={initial.profiles}
      location={{ profile: profileId, page, at }}
      onchange={(r, delay) => {
        picked = r;
        changed(delay);
      }}
    />
  {:else}
    <div class="states">
      <div class="state-tabs" role="tablist" aria-label="States">
        {#each states as s, i (i)}
          <button
            type="button"
            role="tab"
            aria-selected={i === stateIndex}
            class:active={i === stateIndex}
            style="background: {s.bg ?? bg ?? DEFAULT_BG}; color: {s.fg ?? fg ?? DEFAULT_FG}"
            onclick={() => {
              stateIndex = i;
              stateRev++;
            }}>{s.title || `State ${i + 1}`}</button
          >
        {/each}
        <button
          type="button"
          class="btn small quiet"
          onclick={() => {
            states.push({ title: "", bg: null, fg: null, picked: { action: null, nav: null, slider: null } });
            stateIndex = states.length - 1;
            stateRev++;
            changed(0);
          }}>+ State</button
        >
      </div>
      {#key `${stateIndex}:${stateRev}`}
        {@const s = states[stateIndex]}
        <div class="field">
          <label for="state-title">State {stateIndex + 1} title</label>
          <textarea id="state-title" rows="2" bind:value={s.title} oninput={() => changed()}></textarea>
        </div>
        <div class="colors">
          {#each [{ label: "Background", k: "bg" as const }, { label: "Text", k: "fg" as const }] as c (c.k)}
            <div class="field">
              <span id="lbl-s-{c.k}">{c.label}</span>
              <div class="swatch-row">
                <input
                  type="color"
                  aria-labelledby="lbl-s-{c.k}"
                  value={(s[c.k] ?? (c.k === "bg" ? bg ?? DEFAULT_BG : fg ?? DEFAULT_FG)).toLowerCase()}
                  oninput={(e) => {
                    s[c.k] = e.currentTarget.value.toUpperCase();
                    changed(150);
                  }}
                />
                <code>{s[c.k] ?? "key's"}</code>
              </div>
            </div>
          {/each}
        </div>
        <ActionPicker
          value={typeof s.picked === "string" ? { action: null, nav: null, slider: null } : s.picked}
          plugins={initial.plugins}
          allowNav={false}
          allowSlider={false}
          allowSd={false}
          onchange={(r, delay) => {
            s.picked = r;
            changed(delay);
          }}
        />
        {#if states.length > 2}
          <button
            type="button"
            class="link danger-link"
            onclick={() => {
              states.splice(stateIndex, 1);
              stateIndex = 0;
              stateRev++;
              changed(0);
            }}>Remove state {stateIndex + 1}</button
          >
        {/if}
      {/key}
      <div class="field">
        <label for="follow">Follow a live value (optional)</label>
        <input id="follow" bind:value={follow} oninput={() => changed()} placeholder="obs:recording" spellcheck="false" />
        <LiveValuePicker
          plugins={initial.plugins}
          label="Choose a value to follow"
          onpick={(name) => {
            follow = name;
            changed(0);
          }}
        />
        <small>
          With two states, the key shows the second while this value is on (e.g. <code>obs:recording</code>,
          <code>ha:light.desk</code>), instead of switching on each press.
        </small>
      </div>
    </div>
  {/if}

  {#if problem}
    <p class="problem" role="status">{problem}</p>
  {/if}
</form>

<style>
  .kind {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    font-size: 0.92rem;
  }
  .kind label { display: flex; align-items: center; gap: 6px; }
  .kind input { accent-color: var(--apricot-deep); }
  .states {
    display: grid;
    gap: 14px;
    padding: 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
  }
  .state-tabs { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .state-tabs [role="tab"] {
    border: 0;
    border-radius: 8px;
    padding: 6px 10px;
    font: 700 0.8rem/1.1 var(--display);
    white-space: pre-line;
    max-width: 110px;
    opacity: 0.6;
  }
  .state-tabs [role="tab"].active { opacity: 1; outline: 2px solid var(--apricot); outline-offset: 2px; }
  .danger-link { color: var(--danger) !important; justify-self: start; }
</style>
