import { startPlugin, type TestHost } from "@vestra-player/sdk/testing";
import type { Texts } from "../notices.ts";

/**
 * A real `plugin.i18n` (for `new Queue(texts)` outside the plugin) and the
 * host that shows its text values in Simplified Chinese, as the player does.
 */
export async function chineseTexts(): Promise<{ texts: Texts; host: TestHost; stop: () => Promise<void> }> {
  const { plugin, host, stop } = await startPlugin(() => {}, {
    directory: new URL("..", import.meta.url),
    locale: "zh-Hans",
  });
  return { texts: plugin.i18n, host, stop };
}
