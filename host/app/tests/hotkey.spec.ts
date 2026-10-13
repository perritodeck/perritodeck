import { expect, test } from "@playwright/test";
import { chordFromEvent, type Os } from "../src/lib/hotkey";

// The Record button's key press → hotkey text, without a browser. Each case is the
// KeyboardEvent a browser reports on that keyboard layout (code is the US position).

type Mods = Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>>;

function press(code: string, key: string, mods: Mods = {}, os: Os = "windows") {
  return chordFromEvent({ code, key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods }, os);
}

const ctrl = { ctrlKey: true };
const ctrlShift = { ctrlKey: true, shiftKey: true };
const altGr = { ctrlKey: true, altKey: true }; // how Windows reports AltGr

test("letters are the letter the key types, not its US position", () => {
  expect(press("KeyK", "K", { ctrlKey: true, shiftKey: true })).toBe("Ctrl+Shift+K");
  // QWERTZ: Z and Y swap places.
  expect(press("KeyY", "z", ctrl)).toBe("Ctrl+Z");
  expect(press("KeyZ", "y", ctrl)).toBe("Ctrl+Y");
  // AZERTY: A/Q and Z/W swap, M is where US has ';', and the M position types ','.
  expect(press("KeyQ", "a", ctrl)).toBe("Ctrl+A");
  expect(press("KeyW", "z", ctrl)).toBe("Ctrl+Z");
  expect(press("Semicolon", "m", ctrl)).toBe("Ctrl+M");
  expect(press("KeyM", ",", ctrl)).toBe("Ctrl+,");
  // Dvorak: C is in the US I position.
  expect(press("KeyI", "c", ctrl)).toBe("Ctrl+C");
});

test("letter keys that don't type a Latin letter go by position", () => {
  // Russian: Windows treats the "ф" key as A in shortcuts.
  expect(press("KeyA", "ф", ctrl)).toBe("Ctrl+A");
  expect(press("KeyC", "с", ctrl)).toBe("Ctrl+C");
  // Polish AltGr+A types "ą"; the keys pressed were Ctrl+Alt+A.
  expect(press("KeyA", "ą", altGr)).toBe("Ctrl+Alt+A");
  // German AltGr+Q types "@", which the host finds on the same key.
  expect(press("KeyQ", "@", altGr)).toBe("Ctrl+Alt+@");
});

test("punctuation is the character it types", () => {
  expect(press("Equal", "=", ctrl)).toBe("Ctrl+=");
  expect(press("Equal", "+", { ctrlKey: true, shiftKey: true })).toBe("Ctrl+Shift+Plus");
  expect(press("Minus", "-", ctrl)).toBe("Ctrl+Minus");
  expect(press("Slash", "/", ctrl)).toBe("Ctrl+/");
  // German: '-' is in the US '/' position, '+' in the US ']' position, and ü, ß have keys.
  expect(press("Slash", "-", ctrl)).toBe("Ctrl+Minus");
  expect(press("BracketRight", "+", ctrl)).toBe("Ctrl+Plus");
  expect(press("BracketLeft", "ü", ctrl)).toBe("Ctrl+Ü");
  expect(press("Minus", "ß", ctrl)).toBe("Ctrl+ß");
  expect(press("IntlBackslash", "<", ctrl)).toBe("Ctrl+<");
  // A dead key doesn't say what it types: fall back to the US character.
  expect(press("Backquote", "Dead", ctrl)).toBe("Ctrl+`");
});

test("digits, function keys and named keys go by position", () => {
  // AZERTY types '&' on the 1 key without Shift; the shortcut is still Ctrl+1.
  expect(press("Digit1", "&", ctrl)).toBe("Ctrl+1");
  expect(press("Digit7", "/", { ctrlKey: true, shiftKey: true })).toBe("Ctrl+Shift+7");
  expect(press("Numpad5", "5")).toBe("5");
  expect(press("NumpadAdd", "+", ctrl)).toBe("Ctrl+Plus");
  expect(press("F13", "F13")).toBe("F13");
  expect(press("Space", " ", { metaKey: true })).toBe("Super+Space");
  expect(press("Enter", "Enter", { altKey: true })).toBe("Alt+Enter");
  expect(press("ShiftLeft", "Shift", { shiftKey: true })).toBeNull();
});

test("on a Mac, Option keys go by position", () => {
  // Option changes the character (Option+A is "å"), which the host couldn't find.
  expect(press("KeyA", "å", { altKey: true }, "mac")).toBe("Option+A");
  expect(press("Minus", "–", { altKey: true }, "mac")).toBe("Option+Minus");
  expect(press("Slash", "?", { metaKey: true, shiftKey: true }, "mac")).toBe("Shift+Cmd+?");
});

test("Shift and punctuation keep the shifted character on Windows", () => {
  // The host finds the key that types it on the layout in use, Shift included.
  expect(press("Slash", "?", ctrlShift)).toBe("Ctrl+Shift+?");
  expect(press("BracketLeft", "{", ctrlShift)).toBe("Ctrl+Shift+{");
  // German: '_' is Shift on the US '/' key, '?' Shift on the ß key. Their US characters
  // would name other keys there ('/' is Shift+7).
  expect(press("Slash", "_", ctrlShift)).toBe("Ctrl+Shift+_");
  expect(press("Minus", "?", ctrlShift)).toBe("Ctrl+Shift+?");
});

test("on Linux, Shift and punctuation go by the unshifted character", () => {
  // The host only finds characters a key types without Shift, so '?' would fail.
  expect(press("Slash", "?", ctrlShift, "linux")).toBe("Ctrl+Shift+/");
  expect(press("BracketLeft", "{", ctrlShift, "linux")).toBe("Ctrl+Shift+[");
  expect(press("Equal", "+", ctrlShift, "linux")).toBe("Ctrl+Shift+=");
  expect(press("Minus", "_", ctrlShift, "linux")).toBe("Ctrl+Shift+Minus");
  expect(press("Quote", '"', { shiftKey: true }, "linux")).toBe("Shift+'");
  // A key with no US character to fall back on keeps what it typed.
  expect(press("IntlBackslash", ">", ctrlShift, "linux")).toBe("Ctrl+Shift+>");
  // Letters keep the character (the host reads them in either case), on any layout.
  expect(press("KeyY", "Z", ctrlShift, "linux")).toBe("Ctrl+Shift+Z");
  expect(press("BracketLeft", "Ü", ctrlShift, "linux")).toBe("Ctrl+Shift+Ü");
  // Without Shift, punctuation is still the character it types (German '-').
  expect(press("Slash", "-", ctrl, "linux")).toBe("Ctrl+Minus");
  // Digits and named keys are unchanged.
  expect(press("Digit1", "!", ctrlShift, "linux")).toBe("Ctrl+Shift+1");
  expect(press("NumpadAdd", "+", ctrlShift, "linux")).toBe("Ctrl+Shift+Plus");
});
