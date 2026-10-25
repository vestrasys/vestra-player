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
 * @example
 * ```ts
 * import { BINDING_INPUTS } from "@vestra-player/sdk/testing";
 *
 * assert.ok(BINDING_INPUTS.named.includes("PageUp"));
 * assert.ok(BINDING_INPUTS.mouse.includes("Mouse:WheelUp"));
 * ```
 *
 * @since 1.5.14
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
     * Mouse inputs on the video area. `:LeftHalf` and `:RightHalf` tell the
     * halves of the video apart for a double click.
     */
    readonly mouse: readonly ["Mouse:LeftClick", "Mouse:LeftDoubleClick", "Mouse:LeftDoubleClick:LeftHalf", "Mouse:LeftDoubleClick:RightHalf", "Mouse:MiddleClick", "Mouse:X1", "Mouse:X2", "Mouse:WheelUp", "Mouse:WheelDown", "Mouse:WheelLeft", "Mouse:WheelRight"];
}>;
/** Throws `invalid` unless `text` is a canonical binding; returns it unchanged. */
export declare function canonical(text: string): string;
export declare function checkBindings(bindings: readonly string[]): string[];
/** `plugin.bindings` (`bindings.manage`): the table and the user overlay. */
export declare const bindingsApi: (scope: ScopeImpl) => Bindings;
