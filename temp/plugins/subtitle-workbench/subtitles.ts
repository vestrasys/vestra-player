// 实验字幕策略；与最终产品 API 可能差异很大。
export interface Cue {
  start: number;
  end: number;
  text: string;
}
function seconds(time: string): number {
  const parts = /^(\d{2,}):(\d{2}):(\d{2})[,.](\d{3})$/.exec(time);
  if (!parts) throw new Error(`无效字幕时间：${time}`);
  const [hours, minutes, secs, millis] = parts.slice(1).map(Number);
  if (minutes >= 60 || secs >= 60) throw new Error("字幕分秒必须小于 60");
  return hours * 3600 + minutes * 60 + secs + millis / 1000;
}
export function parseSrt(source: string): Cue[] {
  const blocks = source
    .replace(/^\uFEFF/, "")
    .replace(/\r/g, "")
    .trim()
    .split(/\n\s*\n/);
  const cues = blocks.map((block) => {
    const lines = block.split("\n");
    if (/^\d+$/.test(lines[0])) lines.shift();
    const time = /^(\S+)\s+-->\s+(\S+)\s*$/.exec(lines.shift() ?? "");
    if (!time) throw new Error("字幕缺少有效时间轴");
    const start = seconds(time[1]);
    const end = seconds(time[2]);
    const text = lines.join("\n").trim();
    if (end <= start || !text) throw new Error("字幕文本为空或时间区间无效");
    return { start, end, text };
  });
  if (
    !cues.length ||
    cues.some((cue, i) => i > 0 && cue.start < cues[i - 1].start)
  )
    throw new Error("字幕时间轴必须有序且非空");
  return cues;
}
function timestamp(value: number): string {
  const millis = Math.round(Math.max(0, value) * 1000);
  return `${String(Math.floor(millis / 3600000)).padStart(2, "0")}:${String(Math.floor(millis / 60000) % 60).padStart(2, "0")}:${String(Math.floor(millis / 1000) % 60).padStart(2, "0")},${String(millis % 1000).padStart(3, "0")}`;
}
export function serializeSrt(cues: readonly Cue[], delay: number): string {
  if (!Number.isFinite(delay) || Math.abs(delay) > 60)
    throw new Error("字幕延迟必须在 -60 到 60 秒之间");
  return cues
    .filter((cue) => cue.end + delay > 0)
    .map(
      (cue, index) =>
        `${index + 1}\n${timestamp(cue.start + delay)} --> ${timestamp(cue.end + delay)}\n${cue.text}\n`,
    )
    .join("\n");
}
// probe-0.3：模板只引用声明的资源名，真实路径通过 effects.set.files 交给宿主。
export function subtitleFilter(
  resource: string,
  fontSize: number,
  yellow: boolean,
): string {
  if (!Number.isInteger(fontSize) || fontSize < 12 || fontSize > 64)
    throw new Error("字体大小必须为 12–64 的整数");
  if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(resource))
    throw new Error("字幕资源名必须是标识符");
  return `subtitles=filename={{file:${resource}}}:force_style='FontName=Arial,FontSize=${fontSize},PrimaryColour=${yellow ? "&H0000FFFF" : "&H00FFFFFF"},Outline=1,Alignment=2'`;
}
