// 实验任务参数：最终插件 API 与产品交互可能有很大差异。
import type { PlaybackState } from "../../sdk/types.ts";

export interface CaptureSnapshot {
  input: string;
  sessionId: string;
  position: number;
  duration: number | null;
}

export function captureSnapshot(state: PlaybackState): CaptureSnapshot {
  if (!state.path || state.sourceKind === "external")
    throw new Error("当前没有可导出的文件媒体");
  if (!state.sessionId) throw new Error("宿主未提供播放会话标识");
  if (!Number.isFinite(state.position) || state.position < 0)
    throw new Error("播放位置无效");
  return {
    input: state.path,
    sessionId: state.sessionId,
    position: state.position,
    duration: state.duration,
  };
}

export function finiteRange(
  value: number,
  minimum: number,
  maximum: number,
  name: string,
  integer = false,
): number {
  if (
    !Number.isFinite(value) ||
    value < minimum ||
    value > maximum ||
    (integer && !Number.isInteger(value))
  )
    throw new Error(
      `${name} 必须在 ${minimum}–${maximum} 范围内${integer ? "且为整数" : ""}`,
    );
  return value;
}
