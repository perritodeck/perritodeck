// Turn a key press into the hotkey text the host understands, e.g. "Ctrl+Shift+M".
//
// The host reads a character as "the key that types it on the keyboard layout in use", so
// printable keys are recorded by the character they type (KeyboardEvent.key), not by their
// position (KeyboardEvent.code, which is named after a US keyboard). Otherwise Ctrl+Z on a
// German keyboard, where Z sits in the US Y position, would be saved as Ctrl+Y.

// Keys that don't type a character, by position.
const named: Record<string, string> = {
  Enter: "Enter",
  NumpadEnter: "Enter",
  Tab: "Tab",
  Space: "Space",
  Escape: "Esc",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  CapsLock: "CapsLock",
  NumpadSubtract: "Minus",
  NumpadAdd: "Plus",
};

// Punctuation keys by their US character, for when the key doesn't say what it types: a
// dead key (^ on a German keyboard reports "Dead"), or Option held on a Mac.
const punctuation: Record<string, string> = {
  Minus: "Minus",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backquote: "`",
};

// Characters that read better spelled out: "Ctrl+Plus" rather than "Ctrl++".
const spelled: Record<string, string> = { "+": "Plus", "-": "Minus" };

const modifierCodes = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

export const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

/** The character a key types, as the host should read it, or null if it types none. */
function typed(key: string): string | null {
  // One character that isn't a space; "Dead", "Unidentified" and named keys are longer.
  if ([...key].length !== 1 || key.trim() === "") return null;
  if (key in spelled) return spelled[key];
  const upper = key.toUpperCase();
  return [...upper].length === 1 ? upper : key; // "ß" would become "SS"
}

/** The key's name, or null while only modifiers are held. */
function keyName(code: string, key: string, mac: boolean, alt: boolean): string | null {
  if (modifierCodes.has(code)) return null;
  if (code in named) return named[code];
  // Digits by position: shortcuts use the digit keys even where typing a digit needs Shift
  // (AZERTY), and the host sends them that way.
  const digit = /^(?:Digit|Numpad)(\d)$/.exec(code);
  if (digit) return digit[1];
  const f = /^F(\d{1,2})$/.exec(code);
  if (f && Number(f[1]) <= 24) return `F${f[1]}`;
  // Option on a Mac types a different character (Option+A is "å") that the host can't
  // find on the keyboard, so name the key by position instead.
  const char = mac && alt ? null : typed(key);
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) {
    // The character a letter key types if it's ASCII: Z on QWERTZ, A and the ',' in the
    // M position on AZERTY. Otherwise the letter by position, which is how Windows sees
    // shortcuts on layouts without Latin letters: the Russian "ф" key is Ctrl+A there.
    return char && key.charCodeAt(0) < 0x80 ? char : letter[1];
  }
  return char ?? punctuation[code] ?? null;
}

export function chordFromEvent(
  e: { code: string; key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean },
  mac = isMac,
): string | null {
  const k = keyName(e.code, e.key, mac, e.altKey);
  if (!k) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push(mac ? "Option" : "Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push(mac ? "Cmd" : "Super");
  parts.push(k);
  return parts.join("+");
}
