import type { PackageChange, PluginInfo } from "@vestra-player/sdk";

/** What each capability lets a plugin do (J07 detail area). */
export const CAPABILITIES: Record<string, string> = {
  commands: "菜单命令与快捷键",
  "ui.window": "打开工具窗口",
  "ui.mount": "底栏与侧栏面板",
  "ui.dialog": "系统对话框",
  "ui.osd": "屏幕提示",
  "playback.read": "读取播放状态",
  events: "接收实时状态",
  "playback.control": "控制播放",
  "producer.av": "生成音视频流",
  "processing.video": "处理画面",
  "processing.audio": "处理声音",
  "subtitles.read": "读取字幕",
  "subtitles.control": "控制字幕",
  "media.read": "读取画面（截图）",
  "media.control": "截图与导出",
  "source.resolve": "解析网络地址",
  "plugins.manage": "管理插件",
  clipboard: "剪贴板",
  "window.control": "控制主窗口",
  "bindings.manage": "修改快捷键",
};

export const ORIGIN: Record<PluginInfo["origin"], string> = {
  installed: "已安装",
  development: "开发加载",
  trial: "试载",
};

export const STATE: Record<PluginInfo["state"], string> = {
  running: "运行中",
  starting: "启动中",
  disabled: "已停用",
  failed: "启动失败",
  stopped: "已停止",
};

export const CHANGE: Record<PackageChange["status"], string> = {
  preparing: "准备中",
  activating: "激活中",
  restoring: "恢复中",
  committed: "已完成",
  cancelled: "已取消",
  failed: "失败",
  failedDegraded: "失败（已恢复原版本）",
  tooLate: "已无法取消",
};

/** `12 KB`, `3.4 MB`; `—` when unknown. */
export function bytes(size: number | null): string {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** User-facing text for a refused package call. */
export function failureText(error: unknown, info?: PluginInfo): string {
  const code = (error as { code?: string }).code;
  if (code === "invalid") return "插件管理不能停用或卸载自身";
  if (code === "unsupported")
    return info?.origin === "trial" ? "试载中的插件由插件工坊管理" : "开发加载的插件不能卸载或回退";
  if (code === "missing") return "该插件已不存在";
  if (code === "busy") return "正在安装或回退，请稍后再试";
  return (error as Error).message || String(error);
}
