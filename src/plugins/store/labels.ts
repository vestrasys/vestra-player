import type { PackageSource } from "@vestra-player/sdk";

/** Capabilities in plain words (design §2.4); `risky` ones are drawn amber. */
export const CAPABILITIES: Record<string, { text: string; risky?: boolean }> = {
  commands: { text: "添加菜单命令和快捷键" },
  "ui.window": { text: "打开工具窗口" },
  "ui.mount": { text: "在底栏、侧栏或视频背景显示面板" },
  "ui.dialog": { text: "弹出文件选择和确认对话框" },
  "ui.osd": { text: "在视频上显示提示文字" },
  "ui.drop": { text: "接收拖到播放器上的文件" },
  "ui.theme": { text: "切换播放器配色" },
  "ui.settings": { text: "在「选项」窗口添加设置页" },
  "playback.read": { text: "读取正在播放的内容和进度" },
  events: { text: "实时接收播放状态变化" },
  "playback.control": { text: "打开媒体、控制播放、阻止休眠" },
  "producer.av": { text: "向播放器提供自己生成的音视频" },
  "processing.video": { text: "处理画面（滤镜、着色器）" },
  "processing.audio": { text: "处理声音" },
  "subtitles.read": { text: "读取字幕" },
  "subtitles.control": { text: "载入和修改字幕" },
  "media.read": { text: "截图与导出视频片段" },
  "media.control": { text: "截图与导出视频片段" },
  "source.resolve": { text: "解析网络视频地址" },
  "timeline.marks": { text: "在进度条上显示标记" },
  "window.control": { text: "调整主窗口（尺寸、置顶、画中画）" },
  clipboard: { text: "读写剪贴板", risky: true },
  "bindings.manage": { text: "修改所有快捷键", risky: true },
  secrets: { text: "在系统钥匙串保存登录信息", risky: true },
  "plugins.manage": { text: "安装、卸载、停用其它插件", risky: true },
};

export const TRUST_NOTE = "插件是完整的程序，以上权限只限制它使用播放器的功能，不限制它读写文件和联网。";

/** One line per distinct description (media.read and media.control share one). */
export function permissions(capabilities: readonly string[]): { text: string; risky: boolean }[] {
  const seen = new Set<string>();
  const lines: { text: string; risky: boolean }[] = [];
  for (const name of capabilities) {
    const known = CAPABILITIES[name];
    const text = known?.text ?? `未知权限：${name}`;
    if (seen.has(text)) continue;
    seen.add(text);
    lines.push({ text, risky: known?.risky === true });
  }
  return lines;
}

/** Bullet list for a confirmation, at most 20 lines. */
export function permissionList(capabilities: readonly string[]): string {
  const lines = permissions(capabilities);
  if (!lines.length) return "· 不使用播放器的任何功能";
  const shown = lines.slice(0, 20).map((line) => `· ${line.text}`);
  if (lines.length > 20) shown.push(`等 ${lines.length} 项`);
  return shown.join("\n");
}

/** Where a sideloaded or other-source plugin came from. */
export function sourceName(source: PackageSource | null): string {
  switch (source?.kind) {
    case "bundled":
      return "内置";
    case "file":
      return "本地文件";
    case "link":
      return "链接";
    case "workshop":
      return "插件工坊";
    case "repository":
      return `仓库 ${hostOf(source.url)}`;
    default:
      return "未知来源";
  }
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

const pad = (n: number) => String(n).padStart(2, "0");

/** `10:21` today, `10-21 18:02` otherwise (local time). */
export function clock(time: number, now: number): string {
  const at = new Date(time);
  const today = new Date(now);
  const hm = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  return at.toDateString() === today.toDateString() ? hm : `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${hm}`;
}

/** `2026-10-21` from an ISO date. */
export function day(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

/** Cut to `max` characters (confirmation limits count characters). */
export function cut(text: string, max: number): string {
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}
