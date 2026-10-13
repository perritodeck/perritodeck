<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { copyText } from "../lib/clipboard";
  import type { HostStatus, McpSettings, McpView, Network, Update } from "../lib/types";

  let { status, onchange }: { status: HostStatus; onchange: () => void } = $props();
  let error = $state<string | null>(null);
  let autostart = $state<boolean | null>(null);
  // What's saved, and what's being edited.
  let network = $state<Network | null>(null);
  let port = $state("");
  let lan = $state(true);
  let needsRestart = $state(false);
  let checkUpdates = $state<boolean | null>(null);
  let checking = $state(false);
  let checked = $state<{ update: Update | null } | null>(null);
  let detailedLogs = $state<boolean | null>(null);
  /** What was just copied, to say "Copied" next to it. */
  let copied = $state<string | null>(null);
  let mcp = $state<McpView | null>(null);
  let mcpPort = $state("");
  let tokenRenewed = $state(false);

  onMount(async () => {
    try {
      autostart = await api.autostart();
    } catch {
      autostart = null;
    }
    try {
      network = await api.network();
      port = String(network.port);
      lan = network.lan;
      checkUpdates = await api.checkUpdatesEnabled();
    } catch {
      // Older host or mock without these: hide the rows.
    }
    try {
      detailedLogs = await api.detailedLogs();
      mcp = await api.mcp();
      mcpPort = String(mcp.settings.port);
    } catch {
      // The host didn't start: those rows stay hidden.
    }
  });

  async function copy(what: string, text: string | (() => Promise<string>)) {
    await run(async () => {
      await copyText(typeof text === "string" ? text : await text());
      copied = what;
      setTimeout(() => {
        if (copied === what) copied = null;
      }, 2500);
    });
  }

  function saveMcp(change: Partial<McpSettings>) {
    if (!mcp) return;
    const next = { ...mcp.settings, ...change };
    run(async () => {
      mcp = await api.setMcp(next);
      mcpPort = String(mcp.settings.port);
    });
  }

  const mcpPortNumber = $derived(/^\d+$/.test(mcpPort.trim()) ? Number(mcpPort.trim()) : NaN);
  const mcpPortProblem = $derived(
    Number.isInteger(mcpPortNumber) && mcpPortNumber >= 1024 && mcpPortNumber <= 65535
      ? null
      : "Use a number from 1024 to 65535.",
  );
  const mcpPortChanged = $derived(!!mcp && mcpPortNumber !== mcp.settings.port);

  const portNumber = $derived(/^\d+$/.test(port.trim()) ? Number(port.trim()) : NaN);
  const portProblem = $derived(
    Number.isInteger(portNumber) && portNumber >= 1024 && portNumber <= 65535 ? null : "Use a number from 1024 to 65535.",
  );
  const networkChanged = $derived(!!network && (portNumber !== network.port || lan !== network.lan));

  function saveNetwork() {
    if (portProblem) return;
    run(async () => {
      const next = { port: portNumber, lan };
      await api.setNetwork(next);
      network = next;
      needsRestart = true;
    });
  }

  function checkNow() {
    checking = true;
    checked = null;
    run(async () => {
      try {
        checked = { update: await api.checkForUpdates() };
      } finally {
        checking = false;
      }
    });
  }

  async function run(f: () => Promise<void>) {
    error = null;
    try {
      await f();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<h1>Settings</h1>

{#if error}
  <p class="err" role="alert">{error}</p>
{/if}

<dl>
  <dt>Keys</dt>
  <dd>
    <label class="check">
      <input
        type="checkbox"
        checked={status.paused}
        onchange={(e) => run(async () => (await api.setPaused(e.currentTarget.checked), onchange()))}
      />
      Pause all keys
    </label>
    <small>While paused, presses on the tablets do nothing. Also in the tray menu.</small>
  </dd>

  {#if autostart !== null}
    <dt>Start</dt>
    <dd>
      <label class="check">
        <input
          type="checkbox"
          checked={autostart}
          onchange={(e) => {
            const on = e.currentTarget.checked;
            run(async () => {
              await api.setAutostart(on);
              autostart = on;
            });
          }}
        />
        Start PerritoDeck when I log in
      </label>
      <small>It starts in the tray, so the tablets work without opening this window.</small>
    </dd>
  {/if}

  {#if network}
    <dt>Network</dt>
    <dd>
      <div class="row">
        <label for="port">Port</label>
        <input
          id="port"
          class="port"
          inputmode="numeric"
          bind:value={port}
          aria-invalid={!!portProblem}
          aria-describedby="port-help"
        />
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={lan} />
        Let tablets connect over Wi-Fi
      </label>
      <small id="port-help">
        {#if portProblem && port.trim() !== ""}
          <span class="bad">{portProblem}</span>
        {:else if !lan}
          Only tablets on a USB cable can connect. Tablets on Wi-Fi will lose their connection.
        {:else}
          Change the port only if another program uses {network.port}. Tablets on the same Wi-Fi find the new port by
          themselves; if you typed an address into Perrito, change it there too.
        {/if}
      </small>
      {#if networkChanged}
        <span><button class="btn" onclick={saveNetwork} disabled={!!portProblem}>Save network settings</button></span>
      {/if}
      {#if needsRestart && !networkChanged}
        <div class="restart" role="status">
          <span>Saved. PerritoDeck uses the new settings after it restarts.</span>
          <button class="btn small" onclick={() => run(api.restart)}>Restart now</button>
        </div>
      {/if}
    </dd>
  {/if}

  {#if mcp}
    {@const m = mcp}
    <dt id="mcp-heading">AI assistants (MCP)</dt>
    <dd aria-labelledby="mcp-heading">
      <label class="check">
        <input type="checkbox" checked={m.settings.enabled} onchange={(e) => saveMcp({ enabled: e.currentTarget.checked })} />
        Let AI assistants use PerritoDeck
      </label>
      <small>
        Assistants such as Claude Desktop and Claude Code connect over the Model Context Protocol, on this computer only.
        While this is on they can always see your profiles (including what each key does, such as its commands and
        addresses), your tablets and live values. Everything else needs the switches below.
      </small>
      <span role="status" class="mcp-state">
        {#if m.state.state === "running"}
          Running at <span class="mono">{m.state.url}</span>
        {:else if m.state.state === "error"}
          <span class="bad">Couldn’t start: {m.state.message}</span>
        {:else}
          Off
        {/if}
      </span>

      <fieldset class="perms" disabled={!m.settings.enabled}>
        <legend>What assistants may do</legend>
        <label class="check">
          <input type="checkbox" checked={m.settings.press} onchange={(e) => saveMcp({ press: e.currentTarget.checked })} />
          Press keys
        </label>
        <small>Press keys that are already on your profiles, and switch the profile a tablet shows.</small>
        <label class="check">
          <input type="checkbox" checked={m.settings.edit} onchange={(e) => saveMcp({ edit: e.currentTarget.checked })} />
          Edit profiles
        </label>
        <small>Make, change, move and remove keys, pages and profiles. You can undo their edits on the Buttons page.</small>
        <label class="check">
          <input type="checkbox" checked={m.settings.run} onchange={(e) => saveMcp({ run: e.currentTarget.checked })} />
          Run actions
        </label>
        <small>Run any action straight away, including programs and commands, and give keys actions that run programs.</small>
        {#if m.settings.run}
          <p class="warn" role="note">
            An assistant allowed to run actions can do anything you can do on this computer. Turn this on only for an
            assistant you trust, and only while you need it.
          </p>
        {/if}
      </fieldset>

      <div class="row">
        <label for="mcp-port">Port for assistants</label>
        <input id="mcp-port" class="port" inputmode="numeric" bind:value={mcpPort} aria-invalid={!!mcpPortProblem} />
        {#if mcpPortChanged}
          <button class="btn small" disabled={!!mcpPortProblem} onclick={() => saveMcp({ port: mcpPortNumber })}>Save port</button>
        {/if}
      </div>
      {#if mcpPortProblem && mcpPort.trim() !== ""}<small class="bad">{mcpPortProblem}</small>{/if}

      <div class="snippet">
        <div class="snippet-head">
          <span>Claude Desktop and other apps that start a command</span>
          <button class="btn small quiet" onclick={() => copy("stdio", m.snippets.stdioConfig)}>Copy</button>
          {#if copied === "stdio"}<span role="status">Copied</span>{/if}
        </div>
        <small>Add this to the app’s MCP settings (for Claude Desktop, <code>claude_desktop_config.json</code>), then restart it.</small>
        <pre aria-label="Settings for apps that start a command">{m.snippets.stdioConfig}</pre>
      </div>
      <div class="snippet">
        <div class="snippet-head">
          <span>Claude Code</span>
          <button class="btn small quiet" onclick={() => copy("claude", m.snippets.claudeCodeStdio)}>Copy</button>
          {#if copied === "claude"}<span role="status">Copied</span>{/if}
        </div>
        <small>Run this in a terminal. It adds PerritoDeck to Claude Code in every folder, not just the one you’re in.</small>
        <pre aria-label="Claude Code command">{m.snippets.claudeCodeStdio}</pre>
      </div>
      <details>
        <summary>Connect by address instead</summary>
        <small>For apps that connect to a URL. These include the access token: keep them private.</small>
        <div class="snippet">
          <div class="snippet-head">
            <span>Address and token</span>
            <button class="btn small quiet" onclick={() => copy("http", m.snippets.httpConfig)}>Copy</button>
            {#if copied === "http"}<span role="status">Copied</span>{/if}
          </div>
          <pre aria-label="Settings for apps that connect to a URL">{m.snippets.httpConfig}</pre>
        </div>
        <div class="snippet">
          <div class="snippet-head">
            <span>Claude Code, by address</span>
            <button class="btn small quiet" onclick={() => copy("claude-http", m.snippets.claudeCodeHttp)}>Copy</button>
            {#if copied === "claude-http"}<span role="status">Copied</span>{/if}
          </div>
          <pre aria-label="Claude Code command by address">{m.snippets.claudeCodeHttp}</pre>
        </div>
        <span class="row">
          <button
            class="btn"
            onclick={() =>
              run(async () => {
                mcp = await api.newMcpToken();
                tokenRenewed = true;
              })}>Make a new token</button
          >
          {#if tokenRenewed}<span role="status">New token made. Paste the new settings into your assistants.</span>{/if}
        </span>
        <small>Assistants set up with the old token stop working. The command-based setup above needs no token.</small>
      </details>
    </dd>
  {/if}

  <dt>This computer</dt>
  <dd>{status.hostName}</dd>

  <dt>Fingerprint</dt>
  <dd>
    <span class="mono selectable">{status.fingerprint ?? "none (development mode)"}</span>
    <small>Tablets check this when pairing, so nobody else can pretend to be this computer.</small>
  </dd>

  <dt>Tablets connect to</dt>
  <dd>
    {#if status.addresses.length}
      {#each status.addresses as a (a)}<span class="mono selectable">{a}</span>{/each}
    {:else}
      <span>No network found</span>
    {/if}
    <small>Most tablets find this computer by themselves. Type an address in Perrito only if yours doesn’t.</small>
  </dd>

  <dt>USB</dt>
  <dd>
    {#if status.usbAvailable}
      <span>
        {status.usbDevices.length
          ? `${status.usbDevices.length} tablet${status.usbDevices.length === 1 ? "" : "s"} on a cable`
          : "No tablet on a cable right now"}
      </span>
      <small>
        Tablets plugged in with USB debugging on connect over the cable, which is steadier than Wi-Fi. Pair them as
        usual; they switch to the cable by themselves.
      </small>
    {:else}
      <span>Not set up</span>
      <small>
        To use a tablet over a USB cable, install Android’s platform-tools (it includes <code>adb</code>) and restart
        PerritoDeck.
      </small>
    {/if}
  </dd>

  <dt>Profiles</dt>
  <dd>
    <span class="mono selectable">{status.profilesDir}</span>
    <small>One <code>.toml</code> file per profile. Files you add, change or delete here show up straight away.</small>
  </dd>

  <dt>Settings folder</dt>
  <dd>
    <span class="mono selectable">{status.dataDir}</span>
    <small>Holds paired tablets, this computer’s certificate and <code>integrations.toml</code> for OBS and Home Assistant.</small>
    <span><button class="btn" onclick={() => run(api.openDataFolder)}>Open folder</button></span>
  </dd>

  <dt>Integrations</dt>
  <dd>
    {status.integrations.length ? status.integrations.join(", ") : "None set up"}
  </dd>

  <dt>Version</dt>
  <dd>
    <span>{status.version}</span>
    {#if checkUpdates !== null}
      <label class="check">
        <input
          type="checkbox"
          checked={checkUpdates}
          onchange={(e) => {
            const on = e.currentTarget.checked;
            run(async () => {
              await api.setCheckUpdates(on);
              checkUpdates = on;
            });
          }}
        />
        Tell me when a new version is out
      </label>
      <small>PerritoDeck asks GitHub when it starts. It never downloads or installs anything by itself.</small>
      <span class="row">
        <button class="btn" onclick={checkNow} disabled={checking}>{checking ? "Checking…" : "Check now"}</button>
        {#if checked}
          {#if checked.update}
            {@const u = checked.update}
            <span role="status">Version {u.version} is out.</span>
            <button class="btn quiet" onclick={() => run(() => api.openReleasePage(u.url))}>See what's new</button>
          {:else}
            <span role="status">You have the latest version.</span>
          {/if}
        {/if}
      </span>
    {/if}
  </dd>
  <dt>Getting help</dt>
  <dd>
    <span class="row">
      <button class="btn" onclick={() => copy("diagnostics", api.diagnostics)}>Copy diagnostics</button>
      <button class="btn quiet" onclick={() => run(api.openLogsFolder)}>Open logs folder</button>
      {#if copied === "diagnostics"}<span role="status">Copied</span>{/if}
    </span>
    <small>
      Paste the diagnostics into a bug report. They list versions, settings, tablets, plugins and the latest log lines,
      without keys, tokens or passwords. Logs are in <span class="mono selectable">{status.logsDir}</span>.
    </small>
    {#if detailedLogs !== null}
      <label class="check">
        <input
          type="checkbox"
          checked={detailedLogs}
          onchange={(e) => {
            const on = e.currentTarget.checked;
            run(async () => {
              await api.setDetailedLogs(on);
              detailedLogs = on;
            });
          }}
        />
        Detailed logs
      </label>
      <small>Writes much more to the log while you reproduce a problem. Turn it off afterwards.</small>
    {/if}
  </dd>
</dl>

<style>
  h1 { margin-bottom: 24px; }
  dl {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr);
    gap: 22px 24px;
    max-width: 780px;
    margin: 0;
  }
  dt { color: var(--ink-soft); font-weight: 700; }
  dd {
    margin: 0;
    display: grid;
    gap: 6px;
    justify-items: start;
  }
  .mono { font-family: ui-monospace, monospace; font-size: 0.9rem; overflow-wrap: anywhere; }
  small { color: var(--ink-soft); font-size: 0.85rem; max-width: 60ch; }
  .err { color: var(--danger); margin-bottom: 16px; }
  .check { display: flex; align-items: center; gap: 8px; }
  .check input { width: 17px; height: 17px; accent-color: var(--apricot-deep); }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .port {
    width: 8ch;
    font: inherit;
    font-family: ui-monospace, monospace;
    padding: 5px 8px;
    border: 1.5px solid var(--line, #ccc);
    border-radius: 8px;
    background: var(--paper);
    color: var(--ink);
  }
  .port[aria-invalid="true"] { border-color: var(--danger); }
  .bad { color: var(--danger); }
  .mcp-state { font-size: 0.95rem; }
  .perms {
    display: grid;
    gap: 4px;
    margin: 4px 0;
    padding: 10px 14px;
    border: 1.5px solid var(--line, #ccc);
    border-radius: 10px;
    min-width: 0;
  }
  .perms[disabled] { opacity: 0.6; }
  .perms legend { padding: 0 6px; color: var(--ink-soft); font-weight: 700; }
  .perms small { margin: 0 0 6px 25px; }
  .warn {
    margin: 4px 0;
    padding: 8px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--danger) 14%, var(--paper));
    color: var(--ink);
    max-width: 60ch;
  }
  .snippet { display: grid; gap: 4px; width: 100%; max-width: 640px; }
  .snippet-head { display: flex; align-items: center; gap: 10px; font-weight: 600; }
  pre {
    margin: 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--ink) 6%, var(--paper));
    font-size: 0.8rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  details { display: grid; gap: 8px; width: 100%; max-width: 640px; }
  details > :not(summary) { margin-top: 8px; }
  summary { cursor: pointer; color: var(--ink-soft); }
  .restart {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--apricot) 28%, var(--paper));
  }
</style>
