<script lang="ts">
  import { untrack } from "svelte";
  import { DEFAULT_BG, DEFAULT_FG, mediaKeys, obsOps } from "../lib/describe";
  import { api } from "../lib/api";
  import { chordFromEvent } from "../lib/hotkey";
  import type { ActionSpec, KeyFields, KeyView, MediaKey, NavSpec, PluginManifest, Pos, SliderSpec } from "../lib/types";

  let {
    key,
    at,
    page,
    pageNames,
    onsave,
    onclear,
    plugins = [],
  }: {
    key: KeyView | null;
    at: Pos;
    page: string;
    pageNames: string[];
    onsave: (fields: KeyFields) => void;
    onclear: () => void;
    /** Installed plugins, whose actions keys can run. */
    plugins?: PluginManifest[];
  } = $props();

  type Choice =
    | "none"
    | "page"
    | "back"
    | "next"
    | "previous"
    | "hotkey"
    | "text"
    | "media"
    | "openUrl"
    | "openPath"
    | "launch"
    | "command"
    | "obs"
    | "homeAssistant"
    | "multi"
    | "slider"
    | `plugin:${string}`;

  // The draft starts from the key once; the inspector is re-created when the selection
  // (or the key underneath it) changes from outside.
  const initial = untrack(() => ({ key, page, pageNames, plugins }));
  const f = initial.key?.fields ?? { title: null, bg: null, fg: null, action: null, nav: null, icon: null, slider: null };
  const sl = f.slider;
  const a = f.action;
  const nav = f.nav;

  let title = $state(f.title ?? "");
  let icon = $state(f.icon);
  let iconPreview = $state(initial.key?.iconData ?? null);
  let bg = $state(f.bg);
  let fg = $state(f.fg);
  const initialChoice: Choice = sl
    ? "slider"
    : nav
      ? typeof nav === "object"
        ? "page"
        : nav
      : a?.type === "plugin"
        ? `plugin:${a.plugin}/${a.action}`
        : a
          ? a.type
          : "none";
  // Plugin settings by field id.
  let pluginSettings = $state<Record<string, unknown>>(a?.type === "plugin" ? { ...(a.settings ?? {}) } : {});
  const pluginPick = $derived.by(() => {
    if (!choice.startsWith("plugin:")) return null;
    const [pid, aid] = choice.slice(7).split("/");
    const manifest = initial.plugins.find((p) => p.id === pid);
    return { pid, aid, manifest, action: manifest?.actions.find((x) => x.id === aid) };
  });
  let sliderKind = $state<SliderSpec["type"]>(sl?.type ?? "volume");
  let sliderTarget = $state(sl?.type === "homeAssistant" ? sl.entity : sl?.type === "obs" ? sl.input : "");
  let sliderProgram = $state(sl?.type === "command" ? sl.program : "");
  let sliderArgs = $state(sl?.type === "command" ? (sl.args ?? []).join("\n") : "{value}");
  let choice = $state<Choice>(initialChoice);
  let targetPage = $state(
    nav && typeof nav === "object" ? nav.page : (initial.pageNames.find((p) => p !== initial.page) ?? ""),
  );
  let keys = $state(a?.type === "hotkey" ? a.keys : "");
  let text = $state(a?.type === "text" ? a.text : "");
  let media = $state<MediaKey>(a?.type === "media" ? a.key : "playPause");
  let url = $state(a?.type === "openUrl" ? a.url : "https://");
  let path = $state(a?.type === "openPath" ? a.path : "");
  let program = $state(a?.type === "launch" || a?.type === "command" ? a.program : "");
  let args = $state(a?.type === "launch" || a?.type === "command" ? (a.args ?? []).join("\n") : "");
  let timeoutS = $state(a?.type === "command" && a.timeoutMs ? String(a.timeoutMs / 1000) : "");
  let obsOp = $state<keyof typeof obsOps>(a?.type === "obs" ? a.op : "toggleRecord");
  let obsName = $state(a?.type === "obs" && "scene" in a ? a.scene : a?.type === "obs" && "input" in a ? a.input : "");
  let haService = $state(a?.type === "homeAssistant" ? a.service : "");
  let haEntity = $state(a?.type === "homeAssistant" ? (a.entity ?? "") : "");
  let haData = $state(a?.type === "homeAssistant" && a.data != null ? JSON.stringify(a.data) : "");

  let recording = $state(false);
  let problem = $state<string | null>(null);
  const isToggle = (initial.key?.states ?? 0) >= 2;
  const locked = isToggle || initialChoice === "multi";

  /** Build the key from the draft, or explain what's missing. */
  function build(): KeyFields | string {
    let action: ActionSpec | null = null;
    let navOut: NavSpec | null = null;
    let slider: SliderSpec | null = null;
    const lines = (s: string) => s.split("\n").map((l) => l.trim()).filter(Boolean);
    switch (choice) {
      case "none":
        break;
      case "page":
        if (!targetPage) return "Pick the page to open.";
        navOut = { page: targetPage };
        break;
      case "back":
      case "next":
      case "previous":
        navOut = choice;
        break;
      case "hotkey":
        if (!keys.trim()) return "Record or type the keys to press.";
        action = { type: "hotkey", keys: keys.trim() };
        break;
      case "text":
        if (!text) return "Enter the text to type.";
        action = { type: "text", text };
        break;
      case "media":
        action = { type: "media", key: media };
        break;
      case "openUrl":
        if (!/^(https?:\/\/.+|mailto:.+)/.test(url.trim())) return "Enter a web address starting with https://";
        action = { type: "openUrl", url: url.trim() };
        break;
      case "openPath":
        if (!path.trim()) return "Enter the file or folder to open.";
        action = { type: "openPath", path: path.trim() };
        break;
      case "launch":
        if (!program.trim()) return "Enter the program to start.";
        action = { type: "launch", program: program.trim(), args: lines(args) };
        break;
      case "command": {
        if (!program.trim()) return "Enter the program to run.";
        const t = Number(timeoutS);
        if (timeoutS && !(t > 0)) return "The time limit is a number of seconds.";
        action = { type: "command", program: program.trim(), args: lines(args), ...(timeoutS ? { timeoutMs: Math.round(t * 1000) } : {}) };
        break;
      }
      case "obs":
        if ((obsOp === "scene" || obsOp === "toggleMute") && !obsName.trim())
          return obsOp === "scene" ? "Enter the scene's name as OBS shows it." : "Enter the audio source's name as OBS shows it.";
        action =
          obsOp === "scene"
            ? { type: "obs", op: "scene", scene: obsName.trim() }
            : obsOp === "toggleMute"
              ? { type: "obs", op: "toggleMute", input: obsName.trim() }
              : { type: "obs", op: obsOp };
        break;
      case "homeAssistant": {
        if (!/^[a-z_]+\.[a-z0-9_]+$/.test(haService.trim())) return "Enter a service like light.toggle";
        let data: unknown;
        if (haData.trim()) {
          try {
            data = JSON.parse(haData);
          } catch {
            return 'Extra data must be JSON, like {"brightness_pct": 40}';
          }
        }
        action = {
          type: "homeAssistant",
          service: haService.trim(),
          ...(haEntity.trim() ? { entity: haEntity.trim() } : {}),
          ...(data !== undefined ? { data } : {}),
        };
        break;
      }
      case "multi":
        action = key?.fields.action ?? null;
        break;
      default:
        if (choice.startsWith("plugin:") && pluginPick) {
          const settings: Record<string, unknown> = {};
          for (const field of pluginPick.action?.fields ?? []) {
            const v = pluginSettings[field.id] ?? field.default;
            if (v !== undefined && v !== "") settings[field.id] = v;
          }
          action = { type: "plugin", plugin: pluginPick.pid, action: pluginPick.aid, ...(Object.keys(settings).length ? { settings } : {}) };
        }
        break;
      case "slider":
        if (sliderKind === "volume") slider = { type: "volume" };
        else if (sliderKind === "homeAssistant") {
          if (!/^light\.[a-z0-9_]+$/.test(sliderTarget.trim())) return "Enter a light like light.desk";
          slider = { type: "homeAssistant", entity: sliderTarget.trim() };
        } else if (sliderKind === "obs") {
          if (!sliderTarget.trim()) return "Enter the audio source's name as OBS shows it.";
          slider = { type: "obs", input: sliderTarget.trim() };
        } else {
          if (!sliderProgram.trim()) return "Enter the program to run.";
          slider = { type: "command", program: sliderProgram.trim(), args: lines(sliderArgs) };
        }
        break;
    }
    return { title: title === "" ? null : title, bg, fg, action, nav: navOut, icon, slider };
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Save shortly after the last change, so typing makes one edit rather than many. */
  function changed(delay = 350) {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const out = build();
      if (typeof out === "string") {
        problem = out;
        return;
      }
      problem = null;
      onsave(out);
    }, delay);
  }

  function record(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      recording = false;
      return;
    }
    const chord = chordFromEvent(e);
    if (chord) {
      keys = chord;
      recording = false;
      changed(0);
    }
  }

  // "Try it": run the action now. Keyboard actions wait a moment so the person can switch
  // to the window that should receive them.
  let trying = $state<null | { countdown: number } | { result: string; ok: boolean }>(null);
  const tryable = $derived(!["none", "page", "back", "next", "previous", "slider"].includes(choice));
  async function tryIt() {
    const out = build();
    if (typeof out === "string" || !out.action) {
      problem = typeof out === "string" ? out : null;
      return;
    }
    const action = out.action;
    if (["hotkey", "text", "media", "multi"].includes(action.type)) {
      for (let n = 3; n > 0; n--) {
        trying = { countdown: n };
        await new Promise((r) => setTimeout(r, 1000));
      }
    }
    trying = { countdown: 0 };
    try {
      const note = await api.testAction(action);
      trying = note ? { result: `Ran, with a warning: ${note}`, ok: false } : { result: "Done.", ok: true };
    } catch (e) {
      trying = { result: String(e), ok: false };
    }
  }

  const where = $derived(`Row ${at[0] + 1}, column ${at[1] + 1}`);
</script>

<form class="inspector" onsubmit={(e) => e.preventDefault()} aria-label="Key settings">
  <header>
    <p class="where">{where} on {page}</p>
    {#if key}
      <button type="button" class="btn quiet danger small" onclick={onclear}>Remove key</button>
    {/if}
  </header>

  {#if isToggle}
    <p class="note">
      This key is a toggle with {key?.states} states. Toggles are edited in the profile file for now; you can still move
      it.
    </p>
  {/if}

  <div class="field">
    <label for="f1">Title</label>
    <textarea id="f1"
      rows="2"
      bind:value={title}
      oninput={() => changed()}
      disabled={isToggle}
      placeholder={"Two lines\nat most"}></textarea>
    <small>Use <code>{"{time}"}</code> or <code>{"{date}"}</code> for a live clock.</small>
  </div>

  <div class="field">
    <span id="lbl-image">Image</span>
    <div class="image-row">
      {#if iconPreview || icon}
        <span class="thumb" style="background: {bg ?? DEFAULT_BG}">
          {#if iconPreview}<img src={iconPreview} alt="" />{/if}
        </span>
      {/if}
      <button
        type="button"
        class="btn small"
        aria-describedby="lbl-image"
        disabled={isToggle}
        onclick={async () => {
          try {
            const picked = await api.chooseIcon();
            if (picked) {
              icon = picked;
              iconPreview = null;
              changed(0);
            }
          } catch (e) {
            problem = String(e);
          }
        }}>{icon ? "Change…" : "Choose…"}</button
      >
      {#if icon && !isToggle}
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
    <small>PNG or JPEG. It’s copied next to your profiles.</small>
  </div>

  <div class="colors">
    {#each [{ label: "Background", get: () => bg, set: (v: string | null) => (bg = v), def: DEFAULT_BG }, { label: "Text", get: () => fg, set: (v: string | null) => (fg = v), def: DEFAULT_FG }] as c (c.label)}
      <div class="field">
        <span id="lbl-{c.label}">{c.label}</span>
        <div class="swatch-row">
          <input
            type="color"
            aria-labelledby="lbl-{c.label}"
            value={(c.get() ?? c.def).toLowerCase()}
            disabled={isToggle}
            oninput={(e) => {
              c.set(e.currentTarget.value.toUpperCase());
              changed(150);
            }}
          />
          <code>{c.get() ?? "default"}</code>
          {#if c.get() && !isToggle}
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

  <div class="field">
    <label for="f2">When pressed</label>
    <select id="f2" bind:value={choice} onchange={() => changed(0)} disabled={locked}>
      <option value="none">Nothing (a label)</option>
      <optgroup label="Pages">
        <option value="page" disabled={pageNames.length < 2}>Open a page</option>
        <option value="back">Go back</option>
        <option value="next">Next page</option>
        <option value="previous">Previous page</option>
      </optgroup>
      <optgroup label="Keyboard">
        <option value="hotkey">Press a hotkey</option>
        <option value="text">Type text</option>
        <option value="media">Media key</option>
      </optgroup>
      <optgroup label="Open">
        <option value="openUrl">Open a website</option>
        <option value="openPath">Open a file or folder</option>
        <option value="launch">Start a program</option>
        <option value="command">Run a command</option>
      </optgroup>
      <optgroup label="Slider (drag on the tablet)">
        <option value="slider">Slider</option>
      </optgroup>
      <optgroup label="Integrations">
        <option value="obs">OBS Studio</option>
        <option value="homeAssistant">Home Assistant</option>
      </optgroup>
      {#each initial.plugins.filter((p) => p.actions.length) as p (p.id)}
        <optgroup label={p.name}>
          {#each p.actions as act (act.id)}
            <option value={`plugin:${p.id}/${act.id}`}>{act.name}</option>
          {/each}
        </optgroup>
      {/each}
      {#if initialChoice.startsWith("plugin:") && !initial.plugins.some((p) => initialChoice === `plugin:${p.id}/${initialChoice.split("/")[1]}`)}
        <option value={initialChoice}>{initialChoice.slice(7)} (plugin not installed)</option>
      {/if}
      {#if choice === "multi"}
        <option value="multi">Several steps (edit in the profile file)</option>
      {/if}
    </select>
  </div>

  {#if pluginPick}
    {#if !pluginPick.manifest}
      <p class="note">This key uses a plugin that isn’t installed. Add it on the Plugins page.</p>
    {/if}
    {#each pluginPick.action?.fields ?? [] as field (field.id)}
      <div class="field">
        {#if field.type === "checkbox"}
          <label class="check">
            <input
              type="checkbox"
              checked={!!(pluginSettings[field.id] ?? field.default)}
              onchange={(e) => {
                pluginSettings[field.id] = e.currentTarget.checked;
                changed(0);
              }}
            />
            {field.label}
          </label>
        {:else}
          <label for="pf-{field.id}">{field.label}</label>
          {#if field.type === "select"}
            <select
              id="pf-{field.id}"
              value={String(pluginSettings[field.id] ?? field.default ?? field.options?.[0]?.value ?? "")}
              onchange={(e) => {
                pluginSettings[field.id] = e.currentTarget.value;
                changed(0);
              }}
            >
              {#each field.options ?? [] as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
            </select>
          {:else}
            <input
              id="pf-{field.id}"
              inputmode={field.type === "number" ? "decimal" : undefined}
              value={String(pluginSettings[field.id] ?? field.default ?? "")}
              placeholder={field.placeholder ?? ""}
              spellcheck="false"
              oninput={(e) => {
                const v = e.currentTarget.value;
                pluginSettings[field.id] = field.type === "number" && v !== "" && !isNaN(Number(v)) ? Number(v) : v;
                changed();
              }}
            />
          {/if}
        {/if}
      </div>
    {/each}
  {:else if choice === "slider"}
    <div class="field">
      <label for="slider-kind">Slider sets</label>
      <select id="slider-kind" bind:value={sliderKind} onchange={() => changed(0)}>
        <option value="volume">The computer's volume</option>
        <option value="homeAssistant">A Home Assistant light's brightness</option>
        <option value="obs">An OBS audio source's volume</option>
        <option value="command">A program, with the value</option>
      </select>
    </div>
    {#if sliderKind === "homeAssistant" || sliderKind === "obs"}
      <div class="field">
        <label for="slider-target">{sliderKind === "obs" ? "Audio source" : "Light"}</label>
        <input
          id="slider-target"
          bind:value={sliderTarget}
          oninput={() => changed()}
          spellcheck="false"
          placeholder={sliderKind === "obs" ? "Mic/Aux" : "light.desk"}
        />
      </div>
    {:else if sliderKind === "command"}
      <div class="field">
        <label for="slider-program">Program</label>
        <input id="slider-program" bind:value={sliderProgram} oninput={() => changed()} spellcheck="false" placeholder="brightnessctl" />
      </div>
      <div class="field">
        <label for="slider-args">Arguments, one per line</label>
        <textarea id="slider-args" rows="3" bind:value={sliderArgs} oninput={() => changed()} spellcheck="false"></textarea>
        <small><code>{"{value}"}</code> becomes the slider's position, 0 to 100.</small>
      </div>
    {/if}
    <small>Tablets with an older Perrito show this as an ordinary key.</small>
  {:else if choice === "page"}
    <div class="field">
      <label for="f3">Page</label>
      <select id="f3" bind:value={targetPage} onchange={() => changed(0)}>
        {#each pageNames.filter((p) => p !== page) as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    </div>
  {:else if choice === "hotkey"}
    <div class="field">
      <span id="lbl-keys">Keys</span>
      <div class="record-row">
        <input
          aria-labelledby="lbl-keys"
          bind:value={keys}
          oninput={() => changed()}
          onkeydown={record}
          readonly={recording}
          class:recording
          placeholder="Ctrl+Shift+M"
          spellcheck="false"
        />
        <button
          type="button"
          class="btn small"
          class:primary={recording}
          onclick={(e) => {
            recording = !recording;
            (e.currentTarget.previousElementSibling as HTMLInputElement | null)?.focus();
          }}>{recording ? "Press keys…" : "Record"}</button
        >
      </div>
      <small>{recording ? "Press the combination now. Esc cancels." : "Record, or type names joined with +."}</small>
    </div>
  {:else if choice === "text"}
    <div class="field">
      <label for="f4">Text to type</label>
      <textarea id="f4" rows="3" bind:value={text} oninput={() => changed()}></textarea>
    </div>
  {:else if choice === "media"}
    <div class="field">
      <label for="f5">Media key</label>
      <select id="f5" bind:value={media} onchange={() => changed(0)}>
        {#each Object.entries(mediaKeys) as [k, label] (k)}
          <option value={k}>{label}</option>
        {/each}
      </select>
    </div>
  {:else if choice === "openUrl"}
    <div class="field">
      <label for="f6">Web address</label>
      <input id="f6" type="url" bind:value={url} oninput={() => changed()} spellcheck="false" />
    </div>
  {:else if choice === "openPath"}
    <div class="field">
      <label for="f7">File or folder</label>
      <input id="f7" bind:value={path} oninput={() => changed()} spellcheck="false" placeholder="C:\Users\me\Music" />
    </div>
  {:else if choice === "launch" || choice === "command"}
    <div class="field">
      <label for="f8">Program</label>
      <input id="f8" bind:value={program} oninput={() => changed()} spellcheck="false" placeholder="obs64.exe" />
    </div>
    <div class="field">
      <label for="f9">Arguments, one per line</label>
      <textarea id="f9" rows="3" bind:value={args} oninput={() => changed()} spellcheck="false"></textarea>
      {#if choice === "command"}
        <small>Runs directly, not through a shell. The key shows ✓ or ⚠ when it finishes.</small>
      {/if}
    </div>
    {#if choice === "command"}
      <div class="field narrow">
        <label for="f10">Time limit (seconds)</label>
        <input id="f10" inputmode="decimal" bind:value={timeoutS} oninput={() => changed()} placeholder="30" />
      </div>
    {/if}
  {:else if choice === "obs"}
    <div class="field">
      <label for="f11">In OBS</label>
      <select id="f11" bind:value={obsOp} onchange={() => changed(0)}>
        {#each Object.entries(obsOps) as [k, label] (k)}
          <option value={k}>{label}</option>
        {/each}
      </select>
    </div>
    {#if obsOp === "scene" || obsOp === "toggleMute"}
      <div class="field">
        <label for="f12">{obsOp === "scene" ? "Scene" : "Audio source"}</label>
        <input id="f12" bind:value={obsName} oninput={() => changed()} placeholder={obsOp === "scene" ? "Gameplay" : "Mic/Aux"} />
      </div>
    {/if}
  {:else if choice === "homeAssistant"}
    <div class="field">
      <label for="f13">Service</label>
      <input id="f13" bind:value={haService} oninput={() => changed()} spellcheck="false" placeholder="light.toggle" />
    </div>
    <div class="field">
      <label for="f14">Entity (optional)</label>
      <input id="f14" bind:value={haEntity} oninput={() => changed()} spellcheck="false" placeholder="light.desk" />
    </div>
    <div class="field">
      <label for="f15">Extra data as JSON (optional)</label>
      <input id="f15" bind:value={haData} oninput={() => changed()} spellcheck="false" placeholder={'{"brightness_pct": 40}'} />
    </div>
  {/if}

  {#if problem}
    <p class="problem" role="status">{problem}</p>
  {/if}

  {#if tryable}
    <div class="try">
      <button type="button" class="btn small" onclick={tryIt} disabled={trying != null && "countdown" in trying}>Try it</button>
      <span role="status" class:bad={trying && "ok" in trying && !trying.ok}>
        {#if trying && "countdown" in trying}
          {trying.countdown > 0 ? `Switch to the window that should get it… ${trying.countdown}` : "Running…"}
        {:else if trying}
          {trying.result}
        {/if}
      </span>
    </div>
  {/if}
</form>

<style>
  .inspector {
    display: grid;
    gap: 14px;
    align-content: start;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    min-height: 34px;
  }
  .where { color: var(--ink-soft); font-size: 0.85rem; }
  .note {
    font-size: 0.9rem;
    color: var(--ink-soft);
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--paper);
    border: 1px solid var(--line);
  }
  .field {
    display: grid;
    gap: 5px;
  }
  .field > span,
  .field > label { font-weight: 700; font-size: 0.9rem; }
  .field.narrow input { max-width: 8em; }
  small { color: var(--ink-soft); font-size: 0.8rem; }
  input:not([type="color"]),
  textarea,
  select {
    font: inherit;
    color: var(--ink);
    background: var(--paper);
    border: 1.5px solid var(--line);
    border-radius: 10px;
    padding: 7px 10px;
    width: 100%;
    user-select: text;
  }
  textarea { resize: vertical; }
  input:focus-visible,
  textarea:focus-visible,
  select:focus-visible { border-color: var(--apricot); outline: none; }
  :disabled { opacity: 0.55; }
  .colors {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .swatch-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  input[type="color"] {
    width: 34px;
    height: 34px;
    padding: 0;
    border: 1.5px solid var(--line);
    border-radius: 8px;
    background: none;
    cursor: pointer;
  }
  .swatch-row code { font-size: 0.8rem; color: var(--ink-soft); }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--apricot-deep);
    font-size: 0.8rem;
    text-decoration: underline;
  }
  .image-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .thumb {
    width: 40px;
    height: 40px;
    border-radius: 9px;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .thumb img { width: 70%; height: 70%; object-fit: contain; }
  .record-row {
    display: flex;
    gap: 8px;
  }
  .record-row input.recording { border-color: var(--apricot); }
  .btn.small { padding: 5px 12px; font-size: 0.85rem; }
  .problem { color: var(--danger); font-size: 0.9rem; }
  .check { display: flex; align-items: center; gap: 8px; font-weight: 400 !important; }
  .try {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 0.9rem;
    color: var(--ink-soft);
  }
  .try .bad { color: var(--danger); }
</style>
