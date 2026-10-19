// 实验提供者边界：只模拟外部翻译，不模拟解析、宿主处理或会话检查。
import { setTimeout } from "node:timers/promises";
import type { Cue } from "./subtitles.ts";
export interface TranslationProvider {
  translate(cues: readonly Cue[], signal: AbortSignal): Promise<Cue[]>;
}
export class MockTranslationProvider implements TranslationProvider {
  failNext = false;
  async translate(cues: readonly Cue[], signal: AbortSignal): Promise<Cue[]> {
    await setTimeout(300, undefined, { signal });
    if (this.failNext) {
      this.failNext = false;
      throw new Error("Mock 翻译服务暂时不可用");
    }
    return cues.map((cue) => ({ ...cue, text: `[译] ${cue.text}` }));
  }
}
