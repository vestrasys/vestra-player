import type { Host } from "../../sdk/index.ts";
import type { SubtitleState, SubtitleStyle, SubtitleTrack, SubtitleStamp } from "../../sdk/subtitles.ts";
import { open } from "node:fs/promises";
import { createHash } from "node:crypto";

const VIEW = "subtitle-workbench";
const COLORS = ["white", "yellow", "cyan"] as const;
const callbacks = ["load-file", "refresh", "cancel", "select-track", "remove-selected", "reset-style", "change-delay", "change-size", "change-outline", "change-margin", "change-font", "change-color"];
const menus = [["workbench", "字幕工作台…"], ["load", "加载外部 SRT…"], ["disable", "关闭字幕"], ["refresh", "刷新字幕轨"]] as const;
const errorText = (e: unknown) => e instanceof Error ? e.message : String(e);
function same(a: SubtitleTrack | null, b: SubtitleTrack | null): boolean {
  if (!a || !b) return a === b;
  if (a.kind === "embedded" && b.kind === "embedded") return a.id === b.id;
  return a.kind === "external" && b.kind === "external" && a.owner === b.owner && a.resourceId === b.resourceId && a.contentRevision === b.contentRevision;
}
async function readSrt(path: string) {
  if (!/\.srt$/i.test(path)) throw new Error("请选择 UTF-8 .srt 字幕文件。");
  const file = await open(path, "r");
  try {
    const max = 8 * 1024 * 1024;
    const stat = await file.stat();
    if (!stat.isFile() || stat.size < 1 || stat.size > max) throw new Error("SRT 必须为 1 字节至 8 MiB 的普通文件。");
    const buffer = Buffer.alloc(max + 1);
    let length = 0;
    while (length < buffer.length) {
      const result = await file.read(buffer, length, buffer.length - length, null);
      if (!result.bytesRead) break;
      length += result.bytesRead;
    }
    if (length < 1 || length > max) throw new Error("文件为空或读取时超过 8 MiB。");
    const bytes = buffer.subarray(0, length);
    new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return bytes;
  } finally { await file.close(); }
}
export async function activate(host: Host) {
  let live = true, opened = false, busy = false, epoch = 0, serial = Date.now();
  let menuRevision = 1, styleRevision = Date.now(), readRevision = 0, paintRevision = 0;
  let state: SubtitleState | undefined, generation: number | undefined;
  let desired: SubtitleStyle | undefined;
  let status = "选择字幕轨或加载 UTF-8 SRT。";
  let chain = Promise.resolve(), paint = Promise.resolve();
  let styleTimer: ReturnType<typeof setTimeout> | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;
  const offs: (() => void | Promise<void>)[] = [];
  const resources = new Set<string>();
  const id = () => ++serial;
  const menuState = () => ({ enabled: live && !busy, checked: null, busy, reason: busy ? "字幕操作进行中" : null });
  const guard = (ticket: number, source?: number) => {
    if (!live || epoch !== ticket || (source !== undefined && source !== generation)) throw new Error("操作已取消或媒体已切换；请刷新当前字幕状态。");
  };
  function props() {
    const s = state, selected = s?.selection.target ?? null;
    const track = s?.tracks.find(t => same(t.target, selected));
    const style = desired ?? s?.presentation.effective?.style;
    return {
      "status-text": status, "source-text": s ? `媒体版本 ${s.source.generation} · ${s.source.phase} · ${s.tracks.length} 条字幕轨` : "尚无字幕状态",
      "confirmation-text": s ? `轨道：${s.confirmation} · 样式：${s.presentation.confirmation}${s.error ? ` · ${s.error}` : ""}` : "",
      "track-labels": ["关闭字幕", ...(s?.tracks.map(t => `${t.title || (t.target.kind === "embedded" ? `内嵌 ${t.target.id}` : "外部 SRT")}${t.language ? ` · ${t.language}` : ""}`) ?? [])],
      "selected-track": selected && s ? Math.max(0, s.tracks.findIndex(t => same(t.target, selected)) + 1) : 0,
      busy, "can-style": !!track?.styleSupported,
      "can-remove": selected?.kind === "external" && resources.has(selected.resourceId),
      "style-reason": track ? (track.styleSupported ? "样式按 1080p 参考高度缩放。" : track.styleReason ?? "此字幕轨不支持样式。") : "选择字幕后可调整样式与延迟。",
      "font-labels": s?.presentation.fonts.map(f => f.family) ?? [],
      "selected-font": Math.max(0, s?.presentation.fonts.findIndex(f => f.id === style?.fontId) ?? 0),
      "delay-ms": style?.offsetMs ?? 0, "font-size": style?.fontSize ?? 36,
      "outline-size": style?.outline ?? 2, "margin-percent": style?.marginPercent ?? 5,
      "selected-color": Math.max(0, COLORS.indexOf(style?.color ?? "white")),
    };
  }
  function render() {
    const revision = ++paintRevision, values = props();
    paint = paint.catch(() => {}).then(async () => {
      if (!opened) return;
      for (const [key, value] of Object.entries(values)) {
        if (!live || paintRevision !== revision) return;
        await host.ui.set(VIEW, key, value);
      }
    }).catch(e => console.error("subtitle UI:", errorText(e)));
    return paint;
  }
  function accept(s: SubtitleState) {
    if (generation !== undefined && s.source.generation < generation) return;
    if (generation !== s.source.generation) { generation = s.source.generation; desired = undefined; }
    if (state && state.source.generation === s.source.generation && (state.selection.stamp.revision > s.selection.stamp.revision || state.presentation.revision > s.presentation.revision)) return;
    if (state && !same(state.selection.target, s.selection.target)) desired = undefined;
    state = s;
  }
  async function refresh() {
    const revision = ++readRevision, s = await host.subtitles.state();
    if (live && revision === readRevision) { accept(s); await render(); }
    return s;
  }
  async function show() {
    if (!opened) { await host.ui.open(VIEW, "ui/workbench.slint", props(), callbacks); opened = true; }
    else await host.ui.show(VIEW, true);
    await render();
  }
  async function updateMenus() {
    const revision = ++menuRevision;
    for (const [key] of menus) await host.commands.update(key, revision, menuState());
  }
  function run(label: string, action: (ticket: number) => Promise<void>) {
    const ticket = epoch;
    chain = chain.catch(() => {}).then(async () => {
      if (!live || epoch !== ticket) return;
      busy = true; status = label; await render();
      try { await updateMenus(); await action(ticket); }
      catch (e) { status = errorText(e); }
      finally {
        busy = false;
        if (live) { if (!state) await refresh().catch(() => {}); await render(); await updateMenus().catch(e => console.error(errorText(e))); }
      }
    });
    return chain;
  }
  async function recover(operationId: number, action: () => Promise<SubtitleState>) {
    try { return await action(); }
    catch (e) {
      if ((e as { code?: string }).code !== "timeout") throw e;
      for (let n = 0; n < 8 && live; n++) {
        const result = await host.subtitles.operation(operationId);
        if (result.status === "completed") return result.result;
        if (result.status === "failed") throw new Error(`${result.error.code}: ${result.error.message}`);
        if (result.status !== "pending") break;
        await new Promise(resolve => setTimeout(resolve, 250));
      }
      throw new Error(`操作 ${operationId} 结果尚未确认，请刷新；不会自动重放写操作。`);
    }
  }
  async function select(target: SubtitleTrack | null, ticket: number, source?: number) {
    const s = await refresh(); guard(ticket, source ?? s.source.generation);
    const operationId = id(), result = await recover(operationId, () => host.subtitles.select(target, s.selection.stamp, operationId));
    guard(ticket, s.source.generation); accept(result); desired = undefined;
    status = target ? "字幕轨已选择，请查看宿主确认状态。" : "字幕已关闭。";
  }
  async function load(ticket: number) {
    const initial = await refresh(); guard(ticket, initial.source.generation);
    const dialog = await host.dialogs.open({ kind: "openFile", title: "加载 UTF-8 SRT 字幕" });
    guard(ticket, initial.source.generation);
    if (dialog.cancelled || !dialog.paths[0]) { status = "已取消选择字幕文件。"; return; }
    const bytes = await readSrt(dialog.paths[0]); guard(ticket, initial.source.generation);
    const s = await refresh(); guard(ticket, initial.source.generation);
    if (s.source.generation !== initial.source.generation) throw new Error("媒体已切换，请重新加载字幕。");
    const uploadId = id(); let commitSubmitted = false;
    try {
      await host.subtitles.begin(uploadId, s.source.generation, bytes.length);
      for (let offset = 0; offset < bytes.length; offset += 32768) {
        guard(ticket, s.source.generation);
        await host.subtitles.append(uploadId, offset, bytes.subarray(offset, offset + 32768).toString("base64"));
      }
      guard(ticket, s.source.generation);
      const operationId = id(), resourceId = `external-${uploadId}`;
      commitSubmitted = true;
      const result = await recover(operationId, () => host.subtitles.commit({ operationId, uploadId, resourceId, revision: 1, sha256: createHash("sha256").update(bytes).digest("hex"), expected: s.selection.stamp }));
      guard(ticket, s.source.generation); resources.add(resourceId); accept(result); desired = undefined;
      status = `SRT 已加载（${bytes.length} 字节），轨道：${result.confirmation}。`;
    } finally {
      // A commit timeout can still complete. Do not discard its upload underneath it.
      if (!commitSubmitted && live) await host.subtitles.discard(uploadId).catch(() => {});
    }
  }
  function styleChange(key: keyof SubtitleStyle, value: SubtitleStyle[keyof SubtitleStyle]) {
    const s = state, track = s?.tracks.find(t => same(t.target, s.selection.target));
    if (!s || !track?.styleSupported || !s.presentation.fonts[0]) return;
    desired = { ...(desired ?? s.presentation.effective?.style ?? { fontId: s.presentation.fonts[0].id, fontSize: 36, color: "white", outline: 2, marginPercent: 5, offsetMs: 0 }), [key]: value } as SubtitleStyle;
    if (styleTimer) clearTimeout(styleTimer);
    const requested = { ...desired }, target = s.selection.target, stamp = { ...s.selection.stamp };
    void render();
    styleTimer = setTimeout(() => {
      styleTimer = undefined;
      void run("正在应用字幕样式…", async ticket => {
        const latest = await refresh(); guard(ticket, stamp.generation);
        if (!same(target, latest.selection.target) || latest.selection.stamp.revision !== stamp.revision) throw new Error("字幕轨已变化，请在当前轨道重新调整。");
        const operationId = id(), result = await recover(operationId, () => host.subtitles.style({ operationId, revision: ++styleRevision, expected: latest.selection.stamp, style: requested }));
        guard(ticket, stamp.generation); accept(result); status = `字幕样式：${result.presentation.confirmation}。`;
      });
    }, 100);
  }
  function cancel() {
    ++epoch; ++readRevision; desired = undefined;
    if (styleTimer) clearTimeout(styleTimer); styleTimer = undefined;
    status = "已请求取消。已交给宿主的操作可能已生效，请刷新确认；系统选择器请在其窗口取消。"; void render();
  }
  offs.push(host.ui.onCallback(event => {
    if (!live || event.view !== VIEW) return;
    const name = event.callback, value = event.args[0];
    if (name === "cancel") { cancel(); return; }
    if (name === "load-file" && !busy) { void run("请选择 SRT…", load); return; }
    if (name === "refresh" && !busy) { void run("正在刷新…", async () => { await refresh(); desired = undefined; status = "字幕状态已刷新。"; }); return; }
    if (name === "select-track" && !busy && typeof value === "number" && Number.isInteger(value)) {
      const target = value === 0 ? null : state?.tracks[value - 1]?.target, source = state?.source.generation;
      if (target !== undefined) void run("正在切换字幕轨…", ticket => select(target, ticket, source)); return;
    }
    if (name === "remove-selected" && !busy) {
      const target = state?.selection.target, source = state?.source.generation;
      if (target?.kind === "external" && resources.has(target.resourceId)) void run("正在移除字幕…", async ticket => {
        const s = await refresh(); guard(ticket, source);
        if (!same(s.selection.target, target)) throw new Error("选中的字幕已变化，请重新选择。");
        const operationId = id(), result = await recover(operationId, () => host.subtitles.remove(target.resourceId, s.selection.stamp, operationId));
        guard(ticket, s.source.generation); accept(result); resources.delete(target.resourceId); desired = undefined; status = "外部字幕已移除。";
      }); return;
    }
    if (name === "reset-style" && !busy) {
      if (styleTimer) clearTimeout(styleTimer); styleTimer = undefined;
      void run("正在撤销本插件样式…", async ticket => {
        const s = await refresh(); guard(ticket, s.source.generation); const operationId = id();
        const result = await recover(operationId, () => host.subtitles.style({ operationId, revision: ++styleRevision, expected: s.selection.stamp, style: null }));
        guard(ticket, s.source.generation); accept(result); desired = undefined; status = "本插件的样式与延迟已撤销。";
      }); return;
    }
    if (typeof value !== "number" || !Number.isFinite(value)) return;
    const clamp = (min: number, max: number) => Math.round(Math.max(min, Math.min(max, value)));
    if (name === "change-delay") styleChange("offsetMs", clamp(-10000, 10000));
    if (name === "change-size") styleChange("fontSize", clamp(12, 96));
    if (name === "change-outline") styleChange("outline", clamp(0, 8));
    if (name === "change-margin") styleChange("marginPercent", clamp(0, 30));
    if (name === "change-font" && Number.isInteger(value) && state?.presentation.fonts[value]) styleChange("fontId", state.presentation.fonts[value]!.id);
    if (name === "change-color" && Number.isInteger(value) && COLORS[value]) styleChange("color", COLORS[value]!);
  }));
  offs.push(host.commands.onExecute(event => {
    if (!live || busy) return;
    if (event.id === "workbench") void run("正在打开字幕工作台…", async () => { await refresh(); await show(); status = "字幕工作台已就绪。"; });
    if (event.id === "load") void run("请选择 SRT…", async ticket => { await show(); await load(ticket); });
    if (event.id === "disable") void run("正在关闭字幕…", ticket => select(null, ticket));
    if (event.id === "refresh") void run("正在刷新…", async () => { await refresh(); desired = undefined; status = "字幕状态已刷新。"; });
  }));
  try {
    for (const [key, title] of menus) await host.commands.register(key, menuRevision, { title, path: ["字幕", title], shortcut: null }, menuState());
    offs.push(await host.events.subscribe("source.state", data => {
      const event = data as { stamp?: SubtitleStamp; phase?: string };
      let changed = event.phase !== state?.source.phase;
      if (typeof event.stamp?.generation === "number" && generation !== undefined && event.stamp.generation > generation) {
        cancel(); generation = event.stamp.generation; state = undefined; changed = true;
        status = "媒体已切换，正在刷新字幕轨…"; void render();
      }
      if (changed && !busy) void refresh().catch(e => { status = errorText(e); void render(); });
    }));
    offs.push(await host.events.subscribe("playback.backend", () => { if (live && !busy) void refresh().catch(e => { status = errorText(e); void render(); }); }));
    await refresh();
    poll = setInterval(() => { if (live && opened && !busy) void refresh().catch(e => { status = errorText(e); void render(); }); }, 1500);
    poll.unref();
  } catch (e) { live = false; for (const off of offs) await off(); throw e; }
  return async () => {
    live = false; ++epoch; ++readRevision; ++paintRevision;
    if (styleTimer) clearTimeout(styleTimer); if (poll) clearInterval(poll);
    for (const off of offs) await off();
  };
}
