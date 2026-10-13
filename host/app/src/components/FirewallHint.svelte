<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import type { NetworkHint } from "../lib/types";

  // Shown on the Tablets and Pair pages when tablets on Wi-Fi are kept out: a tablet
  // tried Wi-Fi, got no answer and came over the USB cable, or Windows Firewall has no
  // rule letting PerritoDeck in on this network.
  let hint = $state<NetworkHint | null>(null);

  async function refresh() {
    try {
      hint = await api.networkHint();
    } catch {
      hint = null;
    }
  }

  function list(names: string[]): string {
    if (names.length <= 1) return names.join("");
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  }

  onMount(() => {
    refresh();
    const off = api.onHostEvent((e) => {
      if (e.kind === "connected" || e.kind === "paired") refresh();
    });
    // Back from Windows Security or Settings after changing something.
    const back = () => document.visibilityState === "visible" && refresh();
    window.addEventListener("focus", back);
    return () => {
      off.then((f) => f());
      window.removeEventListener("focus", back);
    };
  });
</script>

{#if hint}
  {@const h = hint}
  <section class="hint" aria-labelledby="firewall-hint">
    <h2 id="firewall-hint">
      {h.fellBack.length ? "Tablets can’t reach this computer over Wi-Fi" : "Windows Firewall keeps tablets on Wi-Fi out"}
    </h2>
    <p>
      {#if h.fellBack.length}
        {list(h.fellBack)} couldn’t connect over Wi-Fi, so {h.fellBack.length === 1 ? "it’s" : "they’re"} using the USB cable.
      {/if}
      {#if h.firewallBlocks}
        Windows Firewall doesn’t let PerritoDeck take connections on this {h.publicNetwork ? "public " : ""}network{h.fellBack.length ? "." : ", so only tablets on a USB cable can connect."}
      {:else}
        Check that the tablet is on the same network as this computer. Guest networks and some routers keep devices
        apart{h.os === "windows" ? ", and another security program may block PerritoDeck" : ""}.
      {/if}
    </p>
    {#if h.firewallBlocks}
      <ul>
        <li>
          <strong>Allow PerritoDeck:</strong> open Windows Security, then Firewall &amp; network protection, then
          Allow an app through firewall. Click Change settings, find PerritoDeck and tick
          {h.publicNetwork ? "Public" : "Private"}. If it isn’t listed, add it with Allow another app.
        </li>
        {#if h.publicNetwork}
          <li>
            <strong>Or, on your own network, make it private:</strong> open Settings, then Network &amp; internet,
            then Wi-Fi or Ethernet, pick your network and set Network profile type to Private.
          </li>
        {/if}
      </ul>
    {:else if h.os === "macos"}
      <p>If the firewall is on, open System Settings, then Network, then Firewall, then Options, and allow incoming connections for PerritoDeck.</p>
    {:else if h.os !== "windows"}
      <p>
        If a firewall is on, allow TCP port {h.port}, for example with <code>sudo ufw allow {h.port}/tcp</code>, and
        UDP port 5353 so tablets can find this computer by themselves.
      </p>
    {/if}
    <p class="soft">
      Only using the cable? Turn off <em>Let tablets connect over Wi-Fi</em> in Settings, and this note goes away.
    </p>
  </section>
{/if}

<style>
  .hint {
    display: grid;
    gap: 8px;
    max-width: 860px;
    margin: 0 0 22px;
    padding: 14px 18px;
    border-left: 4px solid var(--apricot-deep);
    border-radius: 10px;
    background: color-mix(in srgb, var(--apricot) 12%, var(--paper));
  }
  h2 { font-size: 1.05rem; margin: 0; }
  p, ul { margin: 0; max-width: 72ch; }
  ul { padding-left: 1.2em; display: grid; gap: 6px; }
  .soft { color: var(--ink-soft); font-size: 0.9rem; }
</style>
