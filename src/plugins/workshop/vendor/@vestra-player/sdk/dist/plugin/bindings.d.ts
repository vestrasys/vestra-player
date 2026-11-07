import type { ScopeImpl } from "./scope.ts";
import type { Bindings } from "./types.ts";
/**
 * The names a binding (`CommandOptions.bindings`, `Bindings.set`) is made
 * of, as the player's binding parser accepts them. A binding is
 * `{ modifier "+" } input`: modifiers in the order of `modifiers`, each at
 * most once, then one input. An input is a name of `letters`, `digits`,
 * `functionKeys`, `named` or `mouse`, or one other printable character that
 * is not a letter, digit, space or `+` (`">"`, `"["`, `"/"`), written as
 * typed, never with `Shift` (`">"`, not `"Shift+."`; `"!"`, not
 * `"Shift+1"`). Anything else fails with code `invalid`. The player's tests
 * check that these agree with the player.
 *
 * @example Check in a test that the plugin's default keys are well formed
 * The test fails on a binding the player would refuse with `invalid`, such
 * as modifiers out of order or a misspelt key name. `isBinding` applies the
 * same grammar as the player's parser: the part after the last `+` is the
 * input, everything before it modifiers in the order of `modifiers`; an
 * input is a listed name or one other printable character, and `Shift`
 * never goes with a digit or such a character.
 * ```ts
 * import assert from "node:assert/strict";
 * import { BINDING_INPUTS } from "@vestra-player/sdk/testing";
 *
 * const { modifiers, letters, digits, functionKeys, named, mouse } = BINDING_INPUTS;
 * const names = new Set([...letters, ...functionKeys, ...named, ...mouse]);
 * function isBinding(binding: string): boolean {
 *   // "Ctrl++" is not a binding: the + key is written "Plus".
 *   const parts = binding.split("+");
 *   const input = parts.pop()!;
 *   const order = parts.map((m) => (modifiers as readonly string[]).indexOf(m));
 *   // Each modifier known, at most once, in the listed order.
 *   if (order.some((at, i) => at < 0 || (i > 0 && at <= order[i - 1]!)))
 *     return false;
 *   if (names.has(input)) return true; // letters are upper case: "A"
 *   const typed = digits.includes(input) || /^[^\p{L}\p{N}\p{C}\s+]$/u.test(input);
 *   // ">" rather than "Shift+.": the character already says Shift.
 *   return typed && !parts.includes("Shift");
 * }
 * for (const binding of ["Ctrl+Shift+PageUp", "Alt+Mouse:WheelUp", ">"])
 *   assert.ok(isBinding(binding), binding);
 * for (const binding of ["Shift+Ctrl+A", "Ctrl+a", "Shift+."])
 *   assert.ok(!isBinding(binding), binding);
 * ```
 *
 * @since 1.5.14
 *
 * @category Testing
 */
export declare const BINDING_INPUTS: Readonly<{
    /**
     * Modifiers, in the order a binding writes them. `Cmd` is ⌘ on macOS and
     * the Windows logo key on Windows; on macOS `Ctrl` also matches ⌘.
     */
    readonly modifiers: readonly ["Ctrl", "Alt", "Shift", "Cmd"];
    /** Letter keys, by key name whatever the case: `A` to `Z`. */
    readonly letters: readonly string[];
    /** Digit keys of the main keyboard: `0` to `9`. */
    readonly digits: readonly string[];
    /** Function keys: `F1` to `F24`. */
    readonly functionKeys: string[];
    /**
     * Other named keys, matched by the physical key. `Plus` is the `+` key
     * however it is typed; the `Numpad…` names are the numeric keypad only,
     * never the main keyboard.
     */
    readonly named: readonly ["Space", "Enter", "Tab", "Escape", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Plus", ...string[], "NumpadPlus", "NumpadMinus", "NumpadMultiply", "NumpadDivide", "NumpadEnter", "NumpadDecimal"];
    /**
     * Mouse inputs on the video area; the side buttons `Mouse:X1` (back) and
     * `Mouse:X2` (forward) anywhere in the main window or the floating sidebar
     * window, except while a menu is open (player 0.6.0).
     * `:LeftHalf` and `:RightHalf` tell the halves of the video apart for a
     * double click.
     */
    readonly mouse: readonly ["Mouse:LeftClick", "Mouse:LeftDoubleClick", "Mouse:LeftDoubleClick:LeftHalf", "Mouse:LeftDoubleClick:RightHalf", "Mouse:MiddleClick", "Mouse:X1", "Mouse:X2", "Mouse:WheelUp", "Mouse:WheelDown", "Mouse:WheelLeft", "Mouse:WheelRight"];
}>;
/** Throws `invalid` unless `text` is a canonical binding; returns it unchanged. */
export declare function canonical(text: string): string;
export declare function checkBindings(bindings: readonly string[]): string[];
/** `plugin.bindings` (`bindings.manage`): the table and the user overlay. */
export declare const bindingsApi: (scope: ScopeImpl) => Bindings;
/**
 * How the player's menus write a canonical binding (`bindings.rs`
 * `label_in`), for `TestHost`'s `commands.keys`: `Ctrl+O` and arrows as
 * ← → ↑ ↓ (`Esc`, `PgUp`, `Num5`); on macOS (`mac`) symbols in Apple's
 * order ⌃⌥⇧⌘, a `Ctrl` binding as ⌘ (it answers to ⌘ there) and ⌃ only
 * next to ⌘. `null` for a binding menus do not show: a mouse binding, and
 * a `Cmd` one outside macOS.
 */
export declare function keyLabel(binding: string, mac: boolean): string | null;
