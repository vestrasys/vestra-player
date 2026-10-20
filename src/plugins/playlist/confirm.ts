import type { Scope } from "../../sdk/plugin/index.ts";

/** A small yes/no tool window; a newer question answers the older one with `false`. */
export function confirmWindow(scope: Scope) {
  const window = scope.windows.create("ui/confirm.slint", { id: "playlist-confirm" });
  let message = "";
  let answer: ((yes: boolean) => void) | null = null;
  const settle = (yes: boolean) => {
    const pending = answer;
    answer = null;
    pending?.(yes);
    return window.hide();
  };
  window.bind(() => ({ message }));
  window.on("accept", () => settle(true));
  window.on("cancel", () => settle(false));
  scope.onDispose(() => answer?.(false));
  return {
    ask(text: string): Promise<boolean> {
      answer?.(false);
      message = text;
      const reply = new Promise<boolean>((resolve) => (answer = resolve));
      scope.update();
      void window.show();
      return reply;
    },
  };
}
