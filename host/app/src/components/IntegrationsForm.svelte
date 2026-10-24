<script lang="ts">
  // Settings → Integrations: OBS Studio, Home Assistant and Bitfocus Companion, saved in
  // integrations.toml. Saved passwords and tokens never come here; the host keeps them.
  // Saved settings are used after a restart, which the form offers.
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { hostOf, validationMessage } from "../lib/integrations";
  import type { Checked, HostStatus, IntegrationInput, IntegrationsView, Secret } from "../lib/types";
  import SecretField from "./SecretField.svelte";

  let { status, onchange }: { status: HostStatus; onchange: () => void | Promise<void> } = $props();

  type Kind = "obs" | "homeAssistant" | "companion";
  const NAMES: Record<Kind, string> = { obs: "OBS", homeAssistant: "Home Assistant", companion: "Companion" };
  const keep: Secret = { kind: "keep" };

  let view = $state<IntegrationsView | null>(null);
  let loadError = $state<string | null>(null);
  let blockError = $state<string | null>(null);
  let confirmStartOver = $state(false);

  // What's being edited.
  let obs = $state({ on: false, url: "", password: keep as Secret });
  let ha = $state({ on: false, url: "", token: keep as Secret });
  let comp = $state({ on: false, host: "", port: "" });

  interface SectionState {
    testing: boolean;
    result: Checked | null;
    /** The draft the result is for. */
    resultFor: string;
    saving: boolean;
    problem: string | null;
    /** The draft just saved, to say "Saved." until it's edited. */
    savedFor: string | null;
    confirmRemove: boolean;
    /** The saved values when this draft started, to notice a change underneath it. */
    base: string;
    changedUnder: boolean;
  }
  const blank = (): SectionState => ({
    testing: false,
    result: null,
    resultFor: "",
    saving: false,
    problem: null,
    savedFor: null,
    confirmRemove: false,
    base: "",
    changedUnder: false,
  });
  let st = $state<Record<Kind, SectionState>>({ obs: blank(), homeAssistant: blank(), companion: blank() });

  const savedOf = (v: IntegrationsView, k: Kind) => JSON.stringify(v[k]);

  function reset(k: Kind, v: IntegrationsView) {
    if (k === "obs") {
      obs.on = v.obs?.enabled ?? false;
      obs.url = v.obs?.url ?? "ws://127.0.0.1:4455";
      obs.password = keep;
    } else if (k === "homeAssistant") {
      ha.on = v.homeAssistant?.enabled ?? false;
      ha.url = v.homeAssistant?.url ?? "";
      ha.token = keep;
    } else {
      comp.on = v.companion?.enabled ?? false;
      comp.host = v.companion?.host ?? "127.0.0.1";
      comp.port = String(v.companion?.port ?? 16622);
    }
    st[k].base = savedOf(v, k);
    st[k].changedUnder = false;
    st[k].confirmRemove = false;
  }

  /** Take a view from the host: clean sections follow it, edited ones keep their draft. */
  function take(v: IntegrationsView, force: Kind[] = []) {
    const before = view;
    view = v;
    for (const k of ["obs", "homeAssistant", "companion"] as Kind[]) {
      if (!before || force.includes(k) || !dirty(k, before)) reset(k, v);
      else st[k].changedUnder = savedOf(v, k) !== st[k].base;
    }
  }

  function dirty(k: Kind, v: IntegrationsView | null = view): boolean {
    if (!v) return false;
    if (k === "obs") {
      const s = v.obs;
      if (obs.on !== (s?.enabled ?? false)) return true;
      return obs.on && (obs.url !== s?.url || obs.password.kind !== "keep");
    }
    if (k === "homeAssistant") {
      const s = v.homeAssistant;
      if (ha.on !== (s?.enabled ?? false)) return true;
      return ha.on && (ha.url !== s?.url || ha.token.kind !== "keep");
    }
    const s = v.companion;
    if (comp.on !== (s?.enabled ?? false)) return true;
    return comp.on && (comp.host !== s?.host || comp.port !== String(s?.port));
  }

  const isDirty = $derived({ obs: dirty("obs", view), homeAssistant: dirty("homeAssistant", view), companion: dirty("companion", view) });

  const problems = $derived({
    obs: obs.on ? validationMessage("obs", { url: obs.url, password: obs.password }, view?.obs ?? null) : null,
    homeAssistant: ha.on ? validationMessage("homeAssistant", { url: ha.url, token: ha.token }, view?.homeAssistant ?? null) : null,
    companion: comp.on ? validationMessage("companion", { host: comp.host, port: comp.port }, null) : null,
  });

  /** "This uses the saved password with …" when a kept secret goes to a new host. */
  const keptNote = $derived({
    obs:
      view?.obs?.hasPassword && obs.password.kind === "keep" && !problems.obs && hostOf(obs.url) !== hostOf(view.obs.url)
        ? `This uses the saved password with ${hostOf(obs.url)}.`
        : null,
    homeAssistant:
      view?.homeAssistant?.hasToken && ha.token.kind === "keep" && !problems.homeAssistant && hostOf(ha.url) !== hostOf(view.homeAssistant.url)
        ? `This sends the saved token to ${hostOf(ha.url)}.`
        : null,
  });

  async function load() {
    try {
      take(await api.integrations());
      loadError = null;
    } catch (e) {
      loadError = String(e);
    }
  }

  onMount(() => {
    load();
  });

  // Companion's connection state is in the status; keep it fresh while it's on.
  onMount(() => {
    const every = setInterval(async () => {
      if (!view?.companion?.enabled) return;
      try {
        await onchange();
      } catch {
        // Keep showing the last state.
      }
    }, 3000);
    return () => clearInterval(every);
  });

  /** The draft as it stands: a test's answer or "Saved." holds only while it's unchanged. */
  function draft(k: Kind): string {
    return JSON.stringify(k === "obs" ? obs : k === "homeAssistant" ? ha : comp);
  }
  const drafts = $derived({ obs: JSON.stringify(obs), homeAssistant: JSON.stringify(ha), companion: JSON.stringify(comp) });

  function untick(k: Kind) {
    // Off shows what's saved, not a half-edited draft.
    if (!view) return;
    if (k === "obs" && !obs.on) {
      obs.url = view.obs?.url ?? obs.url;
      obs.password = keep;
    }
    if (k === "homeAssistant" && !ha.on) {
      ha.url = view.homeAssistant?.url ?? ha.url;
      ha.token = keep;
    }
    if (k === "companion" && !comp.on && view.companion) {
      comp.host = view.companion.host;
      comp.port = String(view.companion.port);
    }
  }

  async function test(k: Kind) {
    st[k].testing = true;
    st[k].result = null;
    st[k].resultFor = draft(k);
    try {
      const t =
        k === "obs"
          ? { kind: "obs" as const, url: obs.url, password: obs.password }
          : k === "homeAssistant"
            ? { kind: "homeAssistant" as const, url: ha.url, token: ha.token }
            : { kind: "companion" as const, host: comp.host, port: comp.port };
      st[k].result = await api.testIntegration($state.snapshot(t));
    } catch (e) {
      st[k].result = { ok: false, message: String(e) };
    } finally {
      st[k].testing = false;
    }
  }

  async function save(k: Kind, change: "use" | "turnOff" | "remove") {
    let input: IntegrationInput;
    if (change !== "use") {
      input = { kind: k, change: { kind: change } } as IntegrationInput;
    } else if (k === "obs") {
      input = { kind: "obs", change: { kind: "use", settings: { url: obs.url, password: obs.password } } };
    } else if (k === "homeAssistant") {
      input = { kind: "homeAssistant", change: { kind: "use", settings: { url: ha.url, token: ha.token } } };
    } else {
      input = { kind: "companion", change: { kind: "use", settings: { host: comp.host, port: comp.port } } };
    }
    st[k].saving = true;
    st[k].problem = null;
    try {
      take(await api.saveIntegration($state.snapshot(input)), [k]);
      st[k].savedFor = draft(k);
      await onchange();
    } catch (e) {
      st[k].problem = String(e);
    } finally {
      st[k].saving = false;
    }
  }

  async function block(f: () => Promise<IntegrationsView | void>) {
    blockError = null;
    try {
      const v = await f();
      if (v) take(v, ["obs", "homeAssistant", "companion"]);
    } catch (e) {
      blockError = String(e);
    }
  }

  const removes: Record<Kind, string> = {
    obs: "Removes OBS's address and password from integrations.toml.",
    homeAssistant: "Removes Home Assistant's address and access token from integrations.toml.",
    companion: "Removes Companion's address from integrations.toml.",
  };
</script>

{#snippet actions(k: Kind, on: boolean, hasSaved: boolean)}
  {@const s = st[k]}
  {#if s.changedUnder && isDirty[k]}
    <p class="note" role="note">integrations.toml changed since you started; saving replaces what's there now.</p>
  {/if}
  <div class="try">
    {#if on}
      <button type="button" class="btn small quiet" onclick={() => test(k)} disabled={s.testing || !!problems[k]} aria-busy={s.testing}>
        {s.testing ? "Testing…" : "Test connection"}
      </button>
    {/if}
    {#if isDirty[k]}
      <button type="button" class="btn small" onclick={() => save(k, on ? "use" : "turnOff")} disabled={s.saving || !!problems[k]}>
        {s.saving ? "Saving…" : "Save"}
      </button>
    {/if}
    {#if s.result && s.resultFor === drafts[k]}
      <span role="status" class:bad={!s.result.ok}>{s.result.ok ? "✓ " : ""}{s.result.message}</span>
    {:else if s.savedFor === drafts[k]}
      <span role="status">Saved.</span>
    {/if}
  </div>
  {#if s.problem}<p class="problem" role="alert">{s.problem}</p>{/if}
  {#if hasSaved}
    {#if s.confirmRemove}
      <div class="confirm">
        <span>{removes[k]}</span>
        <button type="button" class="btn small" onclick={() => save(k, "remove")}>Remove</button>
        <button type="button" class="btn small quiet" onclick={() => (s.confirmRemove = false)}>Cancel</button>
      </div>
    {:else}
      <span><button type="button" class="link" onclick={() => (s.confirmRemove = true)}>Remove saved settings…</button></span>
    {/if}
  {/if}
{/snippet}

<div class="integrations">
  {#if loadError}
    <p class="bad" role="alert">{loadError}</p>
  {:else if view}
    {@const v = view}
    {#if blockError}<p class="bad" role="alert">{blockError}</p>{/if}
    {#if v.restartNeeded}
      <div class="restart" role="status">
        <span>PerritoDeck uses the saved settings after it restarts.</span>
        <button class="btn small" onclick={() => api.restart()}>Restart now</button>
      </div>
    {/if}

    {#if v.mistake}
      <div class="mistake">
        <p class="bad" role="alert">integrations.toml has a mistake: {v.mistake}</p>
        <p>
          {v.runningOlder
            ? "PerritoDeck goes on using the settings it read when it started, until it restarts."
            : "OBS, Home Assistant and Companion stay off until it's fixed."}
        </p>
        <p>Fix it in a text editor, then check again. Or start over.</p>
        <span class="row">
          <button class="btn small" onclick={() => block(api.openIntegrationsFile)}>Open integrations.toml</button>
          <button class="btn small quiet" onclick={() => block(api.integrations)}>Check again</button>
          <button class="btn small quiet" onclick={() => (confirmStartOver = true)}>Start over…</button>
        </span>
        {#if confirmStartOver}
          <div class="confirm">
            <span>
              integrations.toml is renamed to integrations.toml.old, which keeps your passwords; delete it once you're set up
              again. OBS, Home Assistant and Companion are off until then.
            </span>
            <button
              class="btn small"
              onclick={() =>
                block(async () => {
                  confirmStartOver = false;
                  return api.startIntegrationsOver();
                })}>Start over</button
            >
            <button class="btn small quiet" onclick={() => (confirmStartOver = false)}>Cancel</button>
          </div>
        {/if}
      </div>
    {:else}
      {#if v.privacy === "readableByOthers"}
        <div class="warn" role="alert">
          <span>Other accounts on this computer can read integrations.toml, which holds your passwords.</span>
          <button class="btn small" onclick={() => block(api.protectIntegrationsFile)}>Make it private</button>
        </div>
      {/if}

      <fieldset class="form section" id="integrations-obs">
        <legend>OBS Studio</legend>
        <label class="check">
          <input type="checkbox" bind:checked={obs.on} onchange={() => untick("obs")} />
          Use OBS Studio
        </label>
        {#if obs.on || v.obs}
          <div class="field">
            <label for="obs-url">Address</label>
            <input id="obs-url" bind:value={obs.url} disabled={!obs.on} spellcheck="false" aria-invalid={!!problems.obs} />
            <small>In OBS: Tools → WebSocket Server Settings → Show Connect Info.</small>
          </div>
          {#if v.obs?.suggestedUrl && obs.url === v.obs.url}
            {@const s = v.obs.suggestedUrl}
            <p class="note">
              OBS buttons can't use this address. Use {s} instead?
              <button type="button" class="link" onclick={() => (obs.url = s)}>Use it</button>
            </p>
          {/if}
          <SecretField id="obs-password" label="Password" what="password" saved={!!v.obs?.hasPassword} optional disabled={!obs.on} bind:value={obs.password} />
          {#if problems.obs}<p class="problem">{problems.obs}</p>{/if}
          {#if keptNote.obs}<small>{keptNote.obs}</small>{/if}
        {/if}
        {@render actions("obs", obs.on, !!v.obs)}
      </fieldset>

      <fieldset class="form section" id="integrations-ha">
        <legend>Home Assistant</legend>
        <label class="check">
          <input type="checkbox" bind:checked={ha.on} onchange={() => untick("homeAssistant")} />
          Use Home Assistant
        </label>
        {#if ha.on || v.homeAssistant}
          <div class="field">
            <label for="ha-url">Address</label>
            <input
              id="ha-url"
              bind:value={ha.url}
              disabled={!ha.on}
              spellcheck="false"
              placeholder="http://homeassistant.local:8123"
              aria-invalid={!!problems.homeAssistant}
            />
            <small>Use the address you open Home Assistant with.</small>
          </div>
          {#if v.homeAssistant?.suggestedUrl && ha.url === v.homeAssistant.url}
            {@const s = v.homeAssistant.suggestedUrl}
            <p class="note">
              Home Assistant buttons can't use this address. Use {s} instead?
              <button type="button" class="link" onclick={() => (ha.url = s)}>Use it</button>
            </p>
          {/if}
          <SecretField
            id="ha-token"
            label="Access token"
            what="access token"
            saved={!!v.homeAssistant?.hasToken}
            disabled={!ha.on}
            hint="In Home Assistant: your profile → Security → Long-lived access tokens → Create token."
            bind:value={ha.token}
          />
          {#if problems.homeAssistant}<p class="problem">{problems.homeAssistant}</p>{/if}
          {#if keptNote.homeAssistant}<small>{keptNote.homeAssistant}</small>{/if}
        {/if}
        {@render actions("homeAssistant", ha.on, !!v.homeAssistant)}
      </fieldset>

      <fieldset class="form section" id="integrations-companion">
        <legend>Bitfocus Companion</legend>
        <label class="check">
          <input type="checkbox" bind:checked={comp.on} onchange={() => untick("companion")} />
          Use Bitfocus Companion
        </label>
        {#if comp.on || v.companion}
          <div class="pair">
            <div class="field">
              <label for="companion-host">Computer</label>
              <input id="companion-host" bind:value={comp.host} disabled={!comp.on} spellcheck="false" />
            </div>
            <div class="field narrow">
              <label for="companion-port">Satellite port</label>
              <input id="companion-port" type="text" inputmode="numeric" bind:value={comp.port} disabled={!comp.on} />
            </div>
          </div>
          <small>The computer running Companion, and its Satellite port (16622 unless you changed it in Companion).</small>
          {#if problems.companion}<p class="problem">{problems.companion}</p>{/if}
          {#if v.companion?.enabled && status.companion}
            <span class="state" role="status">{status.companion}</span>
          {/if}
        {/if}
        {@render actions("companion", comp.on, !!v.companion)}
      </fieldset>
    {/if}

    <small class="footer">
      {#if v.privacy === "private"}
        Kept in <span class="mono selectable">{v.path}</span>, readable only by your user account.
      {:else if v.privacy === "unknown"}
        Kept in <span class="mono selectable">{v.path}</span>. {"This drive doesn't keep permissions, so other accounts may be able to read it."}
      {:else if v.privacy === "noFile"}
        Kept in <span class="mono selectable">{v.path}</span> once you save.
      {:else}
        Kept in <span class="mono selectable">{v.path}</span>.
      {/if}
      {#if !v.mistake}
        <button type="button" class="link" onclick={() => block(api.openIntegrationsFile)}>Open the file</button>
      {/if}
    </small>
  {/if}
</div>

<style>
  .integrations { display: grid; gap: 14px; width: 100%; max-width: 640px; }
  .section {
    margin: 0;
    padding: 12px 16px 14px;
    border: 1.5px solid var(--line, #ccc);
    border-radius: 12px;
    min-width: 0;
    gap: 10px;
  }
  .section legend { padding: 0 6px; font-weight: 700; color: var(--ink-soft); }
  .pair { display: grid; grid-template-columns: minmax(0, 1fr) 9em; gap: 12px; }
  .mistake { display: grid; gap: 8px; justify-items: start; }
  .mistake p { margin: 0; }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .confirm {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 8px 12px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--danger) 10%, var(--paper));
    font-size: 0.9rem;
  }
  .warn, .restart {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
  }
  .warn { background: color-mix(in srgb, var(--danger) 14%, var(--paper)); }
  .restart { background: color-mix(in srgb, var(--apricot) 28%, var(--paper)); }
  .bad { color: var(--danger); margin: 0; }
  .state { font-size: 0.9rem; }
  .footer { color: var(--ink-soft); font-size: 0.85rem; }
  .mono { font-family: ui-monospace, monospace; overflow-wrap: anywhere; }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--apricot-deep);
    font-size: 0.85rem;
    text-decoration: underline;
  }
  p.note { margin: 0; }
</style>
