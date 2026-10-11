// Turn a key press into the hotkey text the host understands, e.g. "Ctrl+Shift+M".

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
  Minus: "Minus",
  NumpadSubtract: "Minus",
  NumpadAdd: "Plus",
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

/** The key's name, or null while only modifiers are held. */
function keyName(code: string, key: string): string | null {
  if (modifierCodes.has(code)) return null;
  if (code in named) return named[code];
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^(?:Digit|Numpad)(\d)$/.exec(code);
  if (digit) return digit[1];
  const f = /^F(\d{1,2})$/.exec(code);
  if (f && Number(f[1]) <= 24) return `F${f[1]}`;
  return key.length === 1 && key !== " " ? key.toUpperCase() : null;
}

export function chordFromEvent(e: { code: string; key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }): string | null {
  const k = keyName(e.code, e.key);
  if (!k) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push(isMac ? "Option" : "Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push(isMac ? "Cmd" : "Super");
  parts.push(k);
  return parts.join("+");
}
