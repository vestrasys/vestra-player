/**
 * Key and binding text helpers (pure, tested on their own).
 *
 * `fromSlintKey` turns a Slint `key-pressed` event of the capture box into a
 * canonical binding with the same rules as the player's `bindings.rs`
 * (API reference §4.1); the player's `bindings.set` stays the judge.
 */
import type { BindingInfo, BindingRef } from "@vestra-player/sdk";

/** What a Slint key event carries (`event.text`, `event.modifiers.*`). */
export type KeyEvent = { text: string; control: boolean; alt: boolean; shift: boolean; meta: boolean };

/** A captured binding, a key that only ends the capture, or a refusal to show (a Message kind). */
export type Capture = { input: string } | { cancel: true } | { error: "unrecognized" | "invalid" } | null;

/** An input as the pages show it (Slint `Input`): a mouse input after its modifiers, or a key label. */
export type Input = { prefix: string; mouse: number; text: string };
/** `input` of a command is taken by `by` (a command path; Slint `Conflict`). */
export type Conflict = { input: Input; by: string };
/**
 * A line under a page's controls (Slint `Message`): `kind` picks the
 * sentence in ui/parts.slint (Words.message), the rest is its data.
 */
export type Message = { kind: string; text: string; other: string; n: number; input: Input; conflicts: Conflict[]; warn: boolean };
/** Row states of the keyboard page (ui/keys.slint shows their words). */
export const STATE = { default: 0, changed: 1, conflict: 2, ignored: 3, orphan: 4 } as const;
const NO_INPUT: Input = { prefix: "", mouse: -1, text: "" };
export const message = (kind = "", fields: Partial<Message> = {}): Message => ({
  kind,
  text: "",
  other: "",
  n: 0,
  input: NO_INPUT,
  conflicts: [],
  warn: false,
  ...fields,
});

const NAMED: Record<string, string> = {
  " ": "Space",
  "\n": "Enter",
  "\r": "Enter",
  "\t": "Tab",
  "\u{1b}": "Escape",
  "\u{8}": "Backspace",
  "\u{7f}": "Delete",
  "\u{F727}": "Insert",
  "\u{F729}": "Home",
  "\u{F72B}": "End",
  "\u{F72C}": "PageUp",
  "\u{F72D}": "PageDown",
  "\u{F700}": "ArrowUp",
  "\u{F701}": "ArrowDown",
  "\u{F702}": "ArrowLeft",
  "\u{F703}": "ArrowRight",
};

/** US keys and what Shift types on them (Option+Shift digits map to these, §4.1). */
const US_SHIFTED: Record<string, string> = {
  "1": "!", "2": "@", "3": "#", "4": "$", "5": "%", "6": "^", "7": "&", "8": "*", "9": "(", "0": ")",
  "-": "_", "=": "+", "[": "{", "]": "}", "\\": "|", ";": ":", "'": "\"", ",": "<", ".": ">", "/": "?", "`": "~",
};
const US_KEYS = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./";
/** What Option types on each US key (dead keys give their accent). */
const OPTION = "`¡™£¢∞§¶•ªº–≠œ∑´®†¥¨ˆøπ“‘«åß∂ƒ©˙∆˚¬…æΩ≈ç√∫˜µ≤≥÷";
/** What Option+Shift types on each US key. */
const OPTION_SHIFT = "`⁄€‹›ﬁﬂ‡°·‚—±Œ„´‰ˇÁ¨ˆØ∏”’»ÅÍÎÏ˝ÓÔ\u{F8FF}ÒÚÆ¸˛Ç◊ı˜Â¯˘¿";

/** Option characters back to their US key: ø → { key: "o", shifted: false }. */
const OPTION_KEYS = new Map<string, { key: string; shifted: boolean }>();
{
  const keys = [...US_KEYS];
  const plain = [...OPTION];
  const shifted = [...OPTION_SHIFT];
  // The accents of dead keys are the same on both layers; Shift decides.
  for (let i = 0; i < keys.length; i++) {
    if (plain[i] !== "`" && !OPTION_KEYS.has(plain[i]!)) OPTION_KEYS.set(plain[i]!, { key: keys[i]!, shifted: false });
    if (shifted[i] !== "`" && !OPTION_KEYS.has(shifted[i]!)) OPTION_KEYS.set(shifted[i]!, { key: keys[i]!, shifted: true });
  }
}

const join = (mods: { ctrl?: boolean; alt?: boolean; shift?: boolean }, name: string) =>
  `${mods.ctrl ? "Ctrl+" : ""}${mods.alt ? "Alt+" : ""}${mods.shift ? "Shift+" : ""}${name}`;

const symbol = (c: string) => (c === "+" ? "Plus" : c);

/**
 * J01 capture: a Slint key event to a canonical binding.
 * - ⌘ and ⌃ both give `Ctrl` (on macOS `Ctrl+X` answers to either; §4.1).
 * - A modifier alone, a dead key or an empty text keeps the capture waiting.
 * - Plain Esc cancels the capture (Esc itself is bound with the Esc button).
 * - Symbols carry no Shift; `+` is `Plus`; letters are upper case.
 * - Option: US Option characters go back to their key (⌥O `ø` → `Alt+O`,
 *   ⌥⇧. `˘` → `Alt+>`); another symbol typed with Option is that symbol
 *   (German ⌥5 `[` → `[`), as the player matches it first.
 */
export function fromSlintKey(event: KeyEvent): Capture {
  const chars = [...event.text];
  if (chars.length !== 1) return null;
  const c = chars[0]!;
  const code = c.codePointAt(0)!;
  const ctrl = event.control || event.meta;
  const { alt, shift } = event;
  // Slint's codes for the modifier keys themselves (also ⌃P–⌃X, which only ⌘ can capture).
  if (code >= 0x10 && code <= 0x18) return null;
  if (c === "\u{1b}" && !ctrl && !alt && !shift) return { cancel: true };
  // ⌃+letter may arrive as its ASCII control code (⌃I is a Tab code); ⌘ keeps
  // the named key (⌘⌫ is Ctrl+Backspace). Slint's `meta` is ⌃ on macOS.
  if (event.meta && code >= 0x01 && code <= 0x1a) return { input: join({ ctrl, alt, shift }, String.fromCharCode(64 + code)) };
  if (event.meta && code >= 0x1b && code <= 0x1d) return { input: join({ ctrl, alt }, "[\\]"[code - 0x1b]!) };
  if (c === "\u{19}") return { input: join({ ctrl, alt, shift: true }, "Tab") };
  if (NAMED[c]) return { input: join({ ctrl, alt, shift }, NAMED[c]) };
  if (code >= 0xf704 && code <= 0xf71b) return { input: join({ ctrl, alt, shift }, `F${code - 0xf704 + 1}`) };
  if (code < 0x20 || (code >= 0xf700 && code <= 0xf8ff && !(alt && c === "\u{F8FF}"))) return null;
  if (alt) {
    const key = OPTION_KEYS.get(c);
    if (key) {
      if (/[a-z]/.test(key.key)) return { input: join({ ctrl, alt, shift: key.shifted || shift }, key.key.toUpperCase()) };
      return { input: join({ ctrl, alt }, symbol(key.shifted || shift ? US_SHIFTED[key.key]! : key.key)) };
    }
    if (/^[a-z]$/i.test(c)) return { input: join({ ctrl, alt, shift }, c.toUpperCase()) };
    if (/^[0-9]$/.test(c)) return { input: join({ ctrl, alt }, c) };
    if (/^[\p{L}\p{N}\s]$/u.test(c)) return { error: "unrecognized" };
    return { input: join({ ctrl }, symbol(c)) };
  }
  if (/^[a-z]$/i.test(c)) return { input: join({ ctrl, shift }, c.toUpperCase()) };
  if (/^[0-9]$/.test(c) || /^[^\p{L}\p{N}\s]$/u.test(c)) return { input: join({ ctrl }, symbol(c)) };
  return { error: "invalid" };
}

/**
 * Mouse inputs J02 lists, in J02's order (the right button is the menu's).
 * The pages show them by index (ui/parts.slint Words.mouse, translated);
 * `label` is the English name for the mouse OSD message only.
 */
export const MOUSE: { input: string; label: string }[] = [
  // i18n: needs r2-api (the labels are nested in the OSD message of the mouse page)
  { input: "Mouse:LeftClick", label: "Left click" },
  { input: "Mouse:LeftDoubleClick", label: "Left double-click" },
  { input: "Mouse:LeftDoubleClick:LeftHalf", label: "Left double-click (left half)" },
  { input: "Mouse:LeftDoubleClick:RightHalf", label: "Left double-click (right half)" },
  { input: "Mouse:MiddleClick", label: "Middle click" },
  { input: "Mouse:X1", label: "Back button" },
  { input: "Mouse:X2", label: "Forward button" },
  { input: "Mouse:WheelUp", label: "Wheel up" },
  { input: "Mouse:WheelDown", label: "Wheel down" },
  { input: "Mouse:WheelLeft", label: "Wheel left" },
  { input: "Mouse:WheelRight", label: "Wheel right" },
];
/** J02 modifier choices; `""` is none. */
export const MODIFIERS = ["", "Ctrl", "Alt", "Shift", "Cmd"] as const;

const KEY_LABELS: Record<string, string> = {
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Escape: "Esc",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Delete: "Del",
  Plus: "+",
  NumpadPlus: "Num+",
  NumpadMinus: "Num-",
  NumpadMultiply: "Num*",
  NumpadDivide: "Num/",
  NumpadEnter: "NumEnter",
  NumpadDecimal: "Num.",
};

/**
 * Display text of an input the player gave no label for (ignored defaults,
 * orphans, the mouse page): the menu's style (`Ctrl+←`, `Num5`), mouse
 * inputs in (English) words (`Ctrl+Wheel up`; the pages use `inputOf`).
 * Labels of `current` come from the player in the system's style (`⌘O` on
 * macOS; `BindingsModel.labelOf`).
 */
export function label(input: string): string {
  const mouse = input.indexOf("Mouse:");
  if (mouse >= 0) {
    const name = MOUSE.find((m) => m.input === input.slice(mouse))?.label ?? input.slice(mouse + 6);
    return input.slice(0, mouse) + name;
  }
  const parts = input.split("+");
  const name = parts.pop() ?? "";
  const key = KEY_LABELS[name] ?? (/^Numpad[0-9]$/.test(name) ? `Num${name.slice(6)}` : name);
  return [...parts, key].join("+");
}

/**
 * An input for the pages: a listed mouse input as its index (translated in
 * Slint) after its modifiers, anything else as `shown` or `label`.
 */
export function inputOf(input: string, shown?: string): Input {
  const at = input.indexOf("Mouse:");
  const mouse = at >= 0 ? MOUSE.findIndex((m) => m.input === input.slice(at)) : -1;
  if (mouse >= 0) return { prefix: input.slice(0, at), mouse, text: "" };
  return { prefix: "", mouse: -1, text: shown || label(input) };
}

/** True for a mouse binding (`Ctrl+Mouse:WheelUp`). */
export const isMouse = (input: string) => input.includes("Mouse:");

/**
 * Same key for conflicts: on macOS `Ctrl+X` also answers to ⌘X, so `Ctrl+Q`
 * and `Cmd+Q` collide.
 */
export function sameKey(a: string, b: string): boolean {
  if (a === b) return true;
  const norm = (input: string) => {
    const parts = input.split("+");
    const name = parts.pop();
    const mods = new Set(parts.map((m) => (m === "Cmd" ? "Ctrl" : m)));
    return `${["Ctrl", "Alt", "Shift"].filter((m) => mods.has(m)).join("+")}|${name}`;
  };
  return norm(a) === norm(b);
}

/**
 * `Playback ▸ Seek ▸ Forward 5 s`, or the title for commands outside the menu.
 * ▸ (U+25B8) has no emoji presentation; ▶ (U+25B6) showed as a colour emoji.
 */
export const commandText = (info: { path: string[] | null; title: string }) =>
  info.path?.length ? info.path.join(" ▸ ") : info.title;

/** `"<plugin>/<command>"`, the overlay's own key. */
export const refKey = (ref: BindingRef) => `${ref.plugin}/${ref.command}`;
export const sameRef = (a: BindingRef, b: BindingRef) => a.plugin === b.plugin && a.command === b.command;

/** The command an input reaches: the claimer that is not shadowed. */
export function winner(commands: readonly BindingInfo[], input: string, scope: BindingInfo["scope"]) {
  const claimers = commands.filter((c) => c.scope === scope && c.current.includes(input));
  return claimers.find((c) => !c.shadowed.some((s) => s.input === input)) ?? claimers[0] ?? null;
}

/** The player's safe exits, never ignored by J03: Quit (⌘Q) and Exit Fullscreen (Esc). */
export const exempt = (ref: BindingRef) => ref.plugin === "host" && (ref.command === "quit" || ref.command === "exit-fullscreen");
