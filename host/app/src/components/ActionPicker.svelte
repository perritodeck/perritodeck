<script lang="ts" module>
  let counter = 0;
</script>

<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { mediaKeys, obsOps } from "../lib/describe";
  import { api } from "../lib/api";
  import { chordFromEvent } from "../lib/hotkey";
  import type { ActionSpec, MediaKey, NavSpec, PluginField, PluginManifest, ProfileSummary, SdPlugin, SliderSpec } from "../lib/types";

  export type Picked = { action: ActionSpec | null; nav: NavSpec | null; slider: SliderSpec | null };

  let {
    value,
    page = "",
    pageNames = [],
    plugins = [],
    profiles = [],
    allowNav = true,
    allowSlider = true,
    allowSd = true,
    location = null,
    disabled = false,
    onchange,
  }: {
    /** Where the picker starts; it keeps its own draft after that. */
    value: Picked;
    page?: string;
    pageNames?: string[];
    plugins?: PluginManifest[];
    /** Every profile, for "Switch profile". */
    profiles?: ProfileSummary[];
    allowNav?: boolean;
    allowSlider?: boolean;
    /** Offer Stream Deck plugin actions (not on toggle states). */
    allowSd?: boolean;
    /** Where the key is, so a Stream Deck plugin's settings page can be opened for it. */
    location?: { profile: string; page: string; at: [number, number] } | null;
    disabled?: boolean;
    /** The new choice, or a sentence saying what's missing; `delay` is how soon to save. */
    onchange: (result: Picked | string, delay: number) => void;
  } = $props();

  const uid = `ap${++counter}`;

  type Choice =
    | "none"
    | "page"
    | "profile"
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
    | "random"
    | "slider"
    | `plugin:${string}`
    | `sd:${string}`;

  const initial = untrack(() => ({ value, page, pageNames, plugins, allowSd }));
  const a = initial.value.action;
  const nav = initial.value.nav;
  const sl = initial.value.slider;

  const initialChoice: Choice = sl
    ? "slider"
    : nav
      ? typeof nav === "object"
        ? "profile" in nav
          ? "profile"
          : "page"
        : nav
      : a?.type === "plugin"
        ? `plugin:${a.plugin}/${a.action}`
        : a?.type === "sdPlugin"
          ? `sd:${a.plugin}/${a.action}`
          : a
          ? a.type
          : "none";
  let choice = $state<Choice>(initialChoice);
  let pluginSettings = $state<Record<string, unknown>>(
    a?.type === "plugin" ? { ...(a.settings ?? {}) } : sl?.type === "plugin" ? { ...(sl.settings ?? {}) } : {},
  );
  function findPlugin(pick: string) {
    const [pid, aid] = pick.slice(7).split("/");
    const manifest = initial.plugins.find((p) => p.id === pid);
    return { pid, aid, manifest, action: manifest?.actions.find((x) => x.id === aid) };
  }
  const pluginPick = $derived(choice.startsWith("plugin:") ? findPlugin(choice) : null);
  // A slider kind: one of the built-in ones, or `plugin:<id>/<action>`.
  let sliderKind = $state<string>(sl?.type === "plugin" ? `plugin:${sl.plugin}/${sl.action}` : (sl?.type ?? "volume"));
  const sliderPlugin = $derived(choice === "slider" && sliderKind.startsWith("plugin:") ? findPlugin(sliderKind) : null);
  // Stream Deck plugins, fetched when the picker opens.
  let sdPlugins = $state<SdPlugin[]>([]);
  onMount(() => {
    if (!initial.allowSd) return;
    api
      .sdPlugins()
      .then((v) => (sdPlugins = v.installed.filter((p) => p.actions.length)))
      .catch(() => {});
  });
  const sdPick = $derived.by(() => {
    if (!choice.startsWith("sd:")) return null;
    const [pid, aid] = choice.slice(3).split("/");
    const plugin = sdPlugins.find((p) => p.uuid === pid);
    return { pid, aid, plugin, action: plugin?.actions.find((x) => x.uuid === aid) };
  });
  // The plugin's own settings page (property inspector), shown in a frame.
  let sdPage = $state<string | null>(null);
  let sdPageProblem = $state<string | null>(null);
  async function openSdPage() {
    if (!location) return;
    sdPageProblem = null;
    try {
      sdPage = await api.sdSettingsPage(location.profile, location.page, location.at[0], location.at[1]);
    } catch (e) {
      sdPageProblem = String(e);
    }
  }
  let sdSettings = $state(a?.type === "sdPlugin" && a.settings ? JSON.stringify(a.settings, null, 2) : "");
  let sliderFollow = $state(sl?.type === "plugin" ? (sl.follow ?? "") : "");
  const sliderPlugins = initial.plugins
    .map((p) => ({ ...p, actions: p.actions.filter((x) => x.slider) }))
    .filter((p) => p.actions.length);
  let sliderTarget = $state(sl?.type === "homeAssistant" ? sl.entity : sl?.type === "obs" ? sl.input : "");
  let sliderProgram = $state(sl?.type === "command" ? sl.program : "");
  let sliderArgs = $state(sl?.type === "command" ? (sl.args ?? []).join("\n") : "{value}");
  let targetPage = $state(
    nav && typeof nav === "object" && !("profile" in nav)
      ? nav.page
      : (initial.pageNames.find((p) => p !== initial.page) ?? ""),
  );
  // "Switch profile": which one, and optionally which of its pages ("" = its first).
  const navProfile = nav && typeof nav === "object" && "profile" in nav ? nav : null;
  let targetProfile = $state(
    navProfile?.profile ??
      untrack(() => profiles.find((p) => p.id !== location?.profile)?.id ?? profiles[0]?.id ?? ""),
  );
  let targetProfilePage = $state(navProfile?.page ?? "");
  // The chosen profile's pages, fetched when it's picked.
  let profilePages = $state<string[] | null>(null);
  async function loadProfilePages(id: string) {
    profilePages = null;
    if (!id) return;
    try {
      const view = await api.profile(id);
      if (id === targetProfile) profilePages = view.companion ? [] : view.pages.map((p) => p.name);
    } catch {
      if (id === targetProfile) profilePages = [];
    }
  }
  $effect(() => {
    if (choice === "profile") loadProfilePages(targetProfile);
  });
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
  let randomJson = $state(
    a?.type === "random"
      ? JSON.stringify(a.actions, null, 2)
      : '[\n  { "type": "text", "text": "Good game!" },\n  { "type": "text", "text": "Well played!" }\n]',
  );
  let stepsJson = $state(
    a?.type === "multi"
      ? JSON.stringify(a.steps, null, 2)
      : '[\n  { "action": { "type": "hotkey", "keys": "Ctrl+A" }, "delayMs": 100 },\n  { "action": { "type": "hotkey", "keys": "Ctrl+C" } }\n]',
  );

  let recording = $state(false);
  let problem = $state<string | null>(null);

  /** Build the choice from the draft, or explain what's missing. */
  export function build(): Picked | string {
    let action: ActionSpec | null = null;
    let navOut: NavSpec | null = null;
    let slider: SliderSpec | null = null;
    const lines = (s: string) => s.split("\n").map((l) => l.trim()).filter(Boolean);
    // Without the quotes Explorer's "Copy as path" puts around a path.
    const unquote = (s: string) => {
      const t = s.trim();
      return /^"[^"]*"$/.test(t) ? t.slice(1, -1).trim() : t;
    };
    switch (choice) {
      case "none":
        break;
      case "page":
        if (!targetPage) return "Pick the page to open.";
        navOut = { page: targetPage };
        break;
      case "profile":
        if (!targetProfile) return "Pick the profile to switch to.";
        navOut = { profile: targetProfile, ...(targetProfilePage ? { page: targetProfilePage } : {}) };
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
        if (!unquote(path)) return "Enter the file or folder to open.";
        action = { type: "openPath", path: unquote(path) };
        break;
      case "launch":
        if (!unquote(program)) return "Enter the program to start.";
        action = { type: "launch", program: unquote(program), args: lines(args) };
        break;
      case "command": {
        if (!unquote(program)) return "Enter the program to run.";
        const t = Number(timeoutS);
        if (timeoutS && !(t > 0)) return "The time limit is a number of seconds.";
        action = {
          type: "command",
          program: unquote(program),
          args: lines(args),
          ...(timeoutS ? { timeoutMs: Math.round(t * 1000) } : {}),
        };
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
      case "multi": {
        let steps: unknown;
        try {
          steps = JSON.parse(stepsJson);
        } catch {
          return "The steps aren't valid JSON.";
        }
        if (!Array.isArray(steps) || steps.length === 0) return "Add at least one step.";
        for (const st of steps as { action?: { type?: unknown }; delayMs?: unknown }[]) {
          if (!st || typeof st !== "object" || !st.action || typeof st.action.type !== "string")
            return 'Each step needs an "action" with a "type".';
          if (st.delayMs !== undefined && typeof st.delayMs !== "number") return '"delayMs" is a number of milliseconds.';
        }
        action = { type: "multi", steps: steps as { action: ActionSpec; delayMs?: number }[] };
        break;
      }
      case "random": {
        let actions: unknown;
        try {
          actions = JSON.parse(randomJson);
        } catch {
          return "The actions aren't valid JSON.";
        }
        if (!Array.isArray(actions) || actions.length === 0) return "Add at least one action to pick from.";
        for (const x of actions as { type?: unknown }[])
          if (!x || typeof x !== "object" || typeof x.type !== "string") return 'Each action needs a "type".';
        action = { type: "random", actions: actions as ActionSpec[] };
        break;
      }
      case "slider":
        if (sliderKind === "volume") slider = { type: "volume" };
        else if (sliderKind === "homeAssistant") {
          if (!/^light\.[a-z0-9_]+$/.test(sliderTarget.trim())) return "Enter a light like light.desk";
          slider = { type: "homeAssistant", entity: sliderTarget.trim() };
        } else if (sliderKind === "obs") {
          if (!sliderTarget.trim()) return "Enter the audio source's name as OBS shows it.";
          slider = { type: "obs", input: sliderTarget.trim() };
        } else if (sliderPlugin) {
          const settings = pluginSettingsFor(sliderPlugin.action?.fields ?? []);
          slider = {
            type: "plugin",
            plugin: sliderPlugin.pid,
            action: sliderPlugin.aid,
            ...(Object.keys(settings).length ? { settings } : {}),
            ...(sliderFollow ? { follow: sliderFollow } : {}),
          };
        } else {
          if (!unquote(sliderProgram)) return "Enter the program to run.";
          slider = { type: "command", program: unquote(sliderProgram), args: lines(sliderArgs) };
        }
        break;
      default:
        if (sdPick) {
          let settings: unknown;
          if (sdSettings.trim()) {
            try {
              settings = JSON.parse(sdSettings);
            } catch {
              return 'Settings must be JSON, like {"count": 0}';
            }
            if (!settings || typeof settings !== "object" || Array.isArray(settings))
              return 'Settings are a JSON object, like {"count": 0}';
          }
          action = {
            type: "sdPlugin",
            plugin: sdPick.pid,
            action: sdPick.aid,
            ...(settings ? { settings: settings as Record<string, unknown> } : {}),
          };
        } else if (choice.startsWith("plugin:") && pluginPick) {
          const settings = pluginSettingsFor(pluginPick.action?.fields ?? []);
          action = {
            type: "plugin",
            plugin: pluginPick.pid,
            action: pluginPick.aid,
            ...(Object.keys(settings).length ? { settings } : {}),
          };
        }
        break;
    }
    return { action, nav: navOut, slider };
  }

  function pluginSettingsFor(fields: PluginField[]): Record<string, unknown> {
    const settings: Record<string, unknown> = {};
    for (const field of fields) {
      const v = pluginSettings[field.id] ?? field.default;
      if (v !== undefined && v !== "") settings[field.id] = v;
    }
    return settings;
  }

  function changed(delay = 350) {
    const out = build();
    problem = typeof out === "string" ? out : null;
    onchange(out, delay);
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
  const tryable = $derived(
    !["none", "page", "profile", "back", "next", "previous", "slider"].includes(choice) && !choice.startsWith("sd:"),
  );
  async function tryIt() {
    const out = build();
    if (typeof out === "string" || !out.action) {
      problem = typeof out === "string" ? out : null;
      return;
    }
    const action = out.action;
    if (["hotkey", "text", "media", "multi", "random"].includes(action.type)) {
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
</script>

{#snippet pluginFields(fields: PluginField[])}
  {#each fields as field (field.id)}
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
          <label for="{uid}-pf-{field.id}">{field.label}</label>
          {#if field.type === "select"}
            <select
              id="{uid}-pf-{field.id}"
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
              id="{uid}-pf-{field.id}"
              type={field.type === "password" ? "password" : "text"}
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
{/snippet}

<div class="picker form">
  <div class="field">
    <label for="{uid}-f2">When pressed</label>
    <select id="{uid}-f2" bind:value={choice} onchange={() => changed(0)} disabled={disabled}>
      <option value="none">Nothing (a label)</option>
      {#if allowNav}<optgroup label="Pages and profiles">
        <option value="page" disabled={pageNames.length < 2}>Open a page</option>
        <option value="back">Go back</option>
        <option value="next">Next page</option>
        <option value="previous">Previous page</option>
        <option value="profile" disabled={profiles.length < 2 && choice !== "profile"}>Switch profile</option>
      </optgroup>{/if}
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
      {#if allowSlider}
        <optgroup label="Slider (drag on the tablet)">
          <option value="slider">Slider</option>
        </optgroup>
      {/if}
      <optgroup label="Integrations">
        <option value="obs">OBS Studio</option>
        <option value="homeAssistant">Home Assistant</option>
      </optgroup>
      {#each initial.plugins.filter((p) => p.actions.some((x) => !x.slider)) as p (p.id)}
        <optgroup label={p.name}>
          {#each p.actions.filter((x) => !x.slider) as act (act.id)}
            <option value={`plugin:${p.id}/${act.id}`}>{act.name}</option>
          {/each}
        </optgroup>
      {/each}
      {#if initialChoice.startsWith("plugin:") && !initial.plugins.some((p) => initialChoice === `plugin:${p.id}/${initialChoice.split("/")[1]}`)}
        <option value={initialChoice}>{initialChoice.slice(7)} (plugin not installed)</option>
      {/if}
      {#each sdPlugins as p (p.uuid)}
        <optgroup label="{p.name} (Stream Deck plugin)">
          {#each p.actions as act (act.uuid)}
            <option value={`sd:${p.uuid}/${act.uuid}`}>{act.name}</option>
          {/each}
        </optgroup>
      {/each}
      {#if initialChoice.startsWith("sd:") && !sdPlugins.some((p) => initialChoice.startsWith(`sd:${p.uuid}/`))}
        <option value={initialChoice}>{initialChoice.slice(3)}</option>
      {/if}
      <optgroup label="Advanced">
        <option value="multi">Several steps</option>
        <option value="random">Random action</option>
      </optgroup>
    </select>
  </div>

  {#if sdPick}
    {#if sdPlugins.length && !sdPick.plugin}
      <p class="note">This key uses a Stream Deck plugin that isn’t installed. Add it on the Plugins page.</p>
    {:else if sdPick.action?.tooltip}
      <p class="note">{sdPick.action.tooltip}</p>
    {/if}
    {#if sdPick.action?.hasSettingsPage && location}
      <div class="field">
        {#if sdPage}
          <iframe
            class="sd-page"
            title="{sdPick.action.name} settings"
            src={sdPage}
            sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
          ></iframe>
          <span><button type="button" class="btn small quiet" onclick={() => (sdPage = null)}>Close settings</button></span>
          <small>Changes here go straight to the plugin.</small>
        {:else}
          <span><button type="button" class="btn small" onclick={openSdPage}>Open its settings</button></span>
          {#if sdPageProblem}<small class="bad">{sdPageProblem}</small>{/if}
        {/if}
      </div>
    {/if}
    <div class="field">
      <label for="{uid}-sd-settings">Settings, as JSON (optional)</label>
      <textarea
        id="{uid}-sd-settings"
        rows="4"
        class="mono"
        bind:value={sdSettings}
        oninput={() => changed()}
        spellcheck="false"
        placeholder={"{}"}
      ></textarea>
      <small>
        {#if sdPick.action?.hasSettingsPage}
          Most plugins keep their settings themselves (use “Open its settings”). Settings typed here are what the
          plugin starts with.
        {:else}
          The plugin draws this key and remembers its own settings.
        {/if}
      </small>
    </div>
  {:else if pluginPick}
    {#if !pluginPick.manifest}
      <p class="note">This key uses a plugin that isn’t installed. Add it on the Plugins page.</p>
    {/if}
    {@render pluginFields(pluginPick.action?.fields ?? [])}
  {:else if choice === "slider"}
    <div class="field">
      <label for="{uid}-slider-kind">Slider sets</label>
      <select id="{uid}-slider-kind" bind:value={sliderKind} onchange={() => changed(0)}>
        <option value="volume">The computer's volume</option>
        <option value="homeAssistant">A Home Assistant light's brightness</option>
        <option value="obs">An OBS audio source's volume</option>
        <option value="command">A program, with the value</option>
        {#each sliderPlugins as p (p.id)}
          <optgroup label={p.name}>
            {#each p.actions as act (act.id)}
              <option value={`plugin:${p.id}/${act.id}`}>{act.name}</option>
            {/each}
          </optgroup>
        {/each}
        {#if sl?.type === "plugin" && !sliderPlugins.some((p) => p.id === sl.plugin)}
          <option value={`plugin:${sl.plugin}/${sl.action}`}>{sl.plugin}: {sl.action} (plugin not installed)</option>
        {/if}
      </select>
    </div>
    {#if sliderKind === "homeAssistant" || sliderKind === "obs"}
      <div class="field">
        <label for="{uid}-slider-target">{sliderKind === "obs" ? "Audio source" : "Light"}</label>
        <input
          id="{uid}-slider-target"
          bind:value={sliderTarget}
          oninput={() => changed()}
          spellcheck="false"
          placeholder={sliderKind === "obs" ? "Mic/Aux" : "light.desk"}
        />
      </div>
    {:else if sliderPlugin}
      {@render pluginFields(sliderPlugin.action?.fields ?? [])}
      {#if sliderPlugin.manifest?.values.length}
        <div class="field">
          <label for="{uid}-slider-follow">The slider follows</label>
          <select
            id="{uid}-slider-follow"
            value={sliderFollow}
            onchange={(e) => {
              sliderFollow = e.currentTarget.value;
              changed(0);
            }}
          >
            <option value="">Nothing; it stays where it was dragged</option>
            {#each sliderPlugin.manifest.values as v (v.id)}<option value={v.id}>{v.name}</option>{/each}
          </select>
        </div>
      {/if}
    {:else if sliderKind === "command"}
      <div class="field">
        <label for="{uid}-slider-program">Program</label>
        <input id="{uid}-slider-program" bind:value={sliderProgram} oninput={() => changed()} spellcheck="false" placeholder="brightnessctl" />
      </div>
      <div class="field">
        <label for="{uid}-slider-args">Arguments, one per line</label>
        <textarea id="{uid}-slider-args" rows="3" bind:value={sliderArgs} oninput={() => changed()} spellcheck="false"></textarea>
        <small><code>{"{value}"}</code> becomes the slider's position, 0 to 100.</small>
      </div>
    {/if}
    <small>Tablets with an older Perrito show this as an ordinary key.</small>
  {:else if choice === "multi"}
    <div class="field">
      <label for="{uid}-steps">Steps, as JSON</label>
      <textarea id="{uid}-steps" rows="7" bind:value={stepsJson} oninput={() => changed()} spellcheck="false" class="mono"></textarea>
      <small>
        Each step is <code>{'{"action": {…}, "delayMs": 100}'}</code>, with an action as in the profile file, e.g.
        <code>{'{"type": "hotkey", "keys": "Ctrl+C"}'}</code>.
      </small>
    </div>
  {:else if choice === "random"}
    <div class="field">
      <label for="{uid}-random">Actions to pick from, as JSON</label>
      <textarea id="{uid}-random" rows="7" bind:value={randomJson} oninput={() => changed()} spellcheck="false" class="mono"></textarea>
      <small>
        Each press runs one of these, picked at random. Each is an action as in the profile file, e.g.
        <code>{'{"type": "text", "text": "GG"}'}</code>.
      </small>
    </div>
  {:else if choice === "profile"}
    <div class="field">
      <label for="{uid}-profile">Switch to profile</label>
      <select
        id="{uid}-profile"
        bind:value={targetProfile}
        onchange={() => {
          targetProfilePage = "";
          changed(0);
        }}
      >
        {#each profiles as p (p.id)}
          <option value={p.id}>{p.name}{p.id === location?.profile ? " (this one)" : ""}</option>
        {/each}
        {#if targetProfile && !profiles.some((p) => p.id === targetProfile)}
          <option value={targetProfile}>{targetProfile} (not there)</option>
        {/if}
      </select>
    </div>
    <div class="field">
      <label for="{uid}-profile-page">Page in it</label>
      <select id="{uid}-profile-page" bind:value={targetProfilePage} onchange={() => changed(0)}>
        <option value="">Its first page</option>
        {#each profilePages ?? [] as p (p)}
          <option value={p}>{p}</option>
        {/each}
        {#if targetProfilePage && profilePages && !profilePages.includes(targetProfilePage)}
          <option value={targetProfilePage}>{targetProfilePage} (not there)</option>
        {:else if targetProfilePage && !profilePages}
          <option value={targetProfilePage}>{targetProfilePage}</option>
        {/if}
      </select>
      <small>Switches the tablet it’s pressed on, like choosing the profile for it on the Tablets page.</small>
    </div>
  {:else if choice === "page"}
    <div class="field">
      <label for="{uid}-f3">Page</label>
      <select id="{uid}-f3" bind:value={targetPage} onchange={() => changed(0)}>
        {#each pageNames.filter((p) => p !== page) as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    </div>
  {:else if choice === "hotkey"}
    <div class="field">
      <span id="{uid}-lbl-keys">Keys</span>
      <div class="record-row">
        <input
          aria-labelledby="{uid}-lbl-keys"
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
      <label for="{uid}-f4">Text to type</label>
      <!-- The host refuses longer texts (MAX_TEXT_CHARS); maxlength counts UTF-16 units, so it never lets more through. -->
      <textarea id="{uid}-f4" rows="3" maxlength="2000" bind:value={text} oninput={() => changed()}></textarea>
    </div>
  {:else if choice === "media"}
    <div class="field">
      <label for="{uid}-f5">Media key</label>
      <select id="{uid}-f5" bind:value={media} onchange={() => changed(0)}>
        {#each Object.entries(mediaKeys) as [k, label] (k)}
          <option value={k}>{label}</option>
        {/each}
      </select>
    </div>
  {:else if choice === "openUrl"}
    <div class="field">
      <label for="{uid}-f6">Web address</label>
      <input id="{uid}-f6" type="url" bind:value={url} oninput={() => changed()} spellcheck="false" />
    </div>
  {:else if choice === "openPath"}
    <div class="field">
      <label for="{uid}-f7">File or folder</label>
      <input id="{uid}-f7" bind:value={path} oninput={() => changed()} spellcheck="false" placeholder="C:\Users\me\Music" />
    </div>
  {:else if choice === "launch" || choice === "command"}
    <div class="field">
      <label for="{uid}-f8">Program</label>
      <!-- A full path: OBS, for one, isn't found by name. -->
      <input
        id="{uid}-f8"
        bind:value={program}
        oninput={() => changed()}
        spellcheck="false"
        placeholder={choice === "launch" ? "C:\\Program Files\\obs-studio\\bin\\64bit\\obs64.exe" : "hostname"}
      />
    </div>
    <div class="field">
      <label for="{uid}-f9">Arguments, one per line</label>
      <textarea id="{uid}-f9" rows="3" bind:value={args} oninput={() => changed()} spellcheck="false"></textarea>
      {#if choice === "command"}
        <small>Runs directly, not through a shell. The key shows ✓ or ⚠ when it finishes.</small>
      {/if}
    </div>
    {#if choice === "command"}
      <div class="field narrow">
        <label for="{uid}-f10">Time limit (seconds)</label>
        <input id="{uid}-f10" inputmode="decimal" bind:value={timeoutS} oninput={() => changed()} placeholder="10" />
      </div>
    {/if}
  {:else if choice === "obs"}
    <div class="field">
      <label for="{uid}-f11">In OBS</label>
      <select id="{uid}-f11" bind:value={obsOp} onchange={() => changed(0)}>
        {#each Object.entries(obsOps) as [k, label] (k)}
          <option value={k}>{label}</option>
        {/each}
      </select>
    </div>
    {#if obsOp === "scene" || obsOp === "toggleMute"}
      <div class="field">
        <label for="{uid}-f12">{obsOp === "scene" ? "Scene" : "Audio source"}</label>
        <input id="{uid}-f12" bind:value={obsName} oninput={() => changed()} placeholder={obsOp === "scene" ? "Gameplay" : "Mic/Aux"} />
      </div>
    {/if}
  {:else if choice === "homeAssistant"}
    <div class="field">
      <label for="{uid}-f13">Service</label>
      <input id="{uid}-f13" bind:value={haService} oninput={() => changed()} spellcheck="false" placeholder="light.toggle" />
    </div>
    <div class="field">
      <label for="{uid}-f14">Entity (optional)</label>
      <input id="{uid}-f14" bind:value={haEntity} oninput={() => changed()} spellcheck="false" placeholder="light.desk" />
    </div>
    <div class="field">
      <label for="{uid}-f15">Extra data as JSON (optional)</label>
      <input id="{uid}-f15" bind:value={haData} oninput={() => changed()} spellcheck="false" placeholder={'{"brightness_pct": 40}'} />
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
</div>

<style>
  /* Stream Deck settings pages are designed for a dark background. */
  .sd-page {
    width: 100%;
    height: 340px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: #2d2d2d;
    resize: vertical;
  }
  .bad { color: var(--danger); }
</style>
