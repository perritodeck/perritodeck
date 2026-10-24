<script lang="ts">
  // A password or token box that never holds the saved one: the window doesn't get it.
  // It says whether one is saved, and its value is what to do with it: keep the saved
  // one, use what's typed, or remove it.
  import type { Secret } from "../lib/types";

  let {
    id,
    label,
    what,
    saved,
    optional = false,
    disabled = false,
    hint = null,
    value = $bindable(),
  }: {
    id: string;
    label: string;
    /** "password" or "access token", for the notes. */
    what: "password" | "access token";
    saved: boolean;
    optional?: boolean;
    disabled?: boolean;
    hint?: string | null;
    value: Secret;
  } = $props();

  let text = $state("");
  // Put back to "keep" from outside (a reset, a save): the box empties too.
  $effect(() => {
    if (value.kind === "keep" && text !== "") text = "";
  });

  const savedNote = $derived(what === "password" ? "A password is saved." : "An access token is saved.");

  function typed(t: string) {
    text = t;
    value = t === "" ? { kind: "keep" } : { kind: "set", value: t };
  }
</script>

<div class="field">
  <label for={id}>{label}{optional ? " (optional)" : ""}</label>
  <input
    {id}
    type="password"
    autocomplete="new-password"
    spellcheck="false"
    value={text}
    placeholder={saved && value.kind !== "clear" ? "Saved (type to replace)" : ""}
    disabled={disabled || value.kind === "clear"}
    oninput={(e) => typed(e.currentTarget.value)}
  />
  {#if hint}<small>{hint}</small>{/if}
  {#if saved && value.kind === "clear"}
    <small>
      The saved {what} is removed when you save.
      <button type="button" class="link" onclick={() => (value = { kind: "keep" })}>Keep it</button>
    </small>
  {:else if saved}
    <small>
      {savedNote} Type a new one to replace it.
      {#if optional && !disabled && value.kind === "keep"}
        <button type="button" class="link" onclick={() => (value = { kind: "clear" })}>Remove the saved {what}</button>
      {/if}
    </small>
  {/if}
</div>
