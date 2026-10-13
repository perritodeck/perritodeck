// Turn a key press into the hotkey text the host understands, e.g. "Ctrl+Shift+M".
//
// The host reads a character as "the key that types it on the keyboard layout in use", so
// printable keys are recorded by the character they type (KeyboardEvent.key), not by their
// position (KeyboardEvent.code, which is named after a US keyboard). Otherwise Ctrl+Z on a
// German keyboard, where Z sits in the US Y position, would be saved as Ctrl+Y. The one
// exception is a punctuation key with Shift held on Linux (see keyName).

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
// dead key (^ on a German keyboard reports "Dead"), Option held on a Mac, or Shift held on
// Linux.
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

/** The system the app runs on, which is the one the host presses the keys on (anything
 *  that isn't a Mac or Linux counts as Windows). */
export type Os = "mac" | "linux" | "windows";

const platform = typeof navigator !== "undefined" ? navigator.platform : "";
export const currentOs: Os = /Mac/.test(platform) ? "mac" : /Linux/.test(platform) ? "linux" : "windows";

/** The character a key types, as the host should read it, or null if it types none. */
function typed(key: string): string | null {
  // One character that isn't a space; "Dead", "Unidentified" and named keys are longer.
  if ([...key].length !== 1 || key.trim() === "") return null;
  if (key in spelled) return spelled[key];
  const upper = key.toUpperCase();
  return [...upper].length === 1 ? upper : key; // "ß" would become "SS"
}

/** The key's name, or null while only modifiers are held. */
function keyName(code: string, key: string, os: Os, alt: boolean, shift: boolean): string | null {
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
  const char = os === "mac" && alt ? null : typed(key);
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) {
    // The character a letter key types if it's ASCII: Z on QWERTZ, A and the ',' in the
    // M position on AZERTY. Otherwise the letter by position, which is how Windows sees
    // shortcuts on layouts without Latin letters: the Russian "ф" key is Ctrl+A there.
    return char && key.charCodeAt(0) < 0x80 ? char : letter[1];
  }
  // With Shift held, a punctuation key types its shifted character: '?' on the '/' key. On
  // Linux the host only finds characters that a key types without Shift (it looks letters
  // up in lower case, so "Ü" is fine), so it couldn't press Ctrl+Shift+?; name the key by
  // position there instead, Ctrl+Shift+/. Windows and macOS find a shifted character's key
  // on any layout, so they keep the character, which stays right on other layouts too: a
  // German keyboard has '_' on the US '/' key.
  if (shift && os === "linux" && key.toLowerCase() === key.toUpperCase()) {
    return punctuation[code] ?? char;
  }
  return char ?? punctuation[code] ?? null;
}

export function chordFromEvent(
  e: { code: string; key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean },
  os = currentOs,
): string | null {
  const k = keyName(e.code, e.key, os, e.altKey, e.shiftKey);
  if (!k) return null;
  const mac = os === "mac";
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push(mac ? "Option" : "Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push(mac ? "Cmd" : "Super");
  parts.push(k);
  return parts.join("+");
}
