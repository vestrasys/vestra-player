// 仅用于 Slint 实验；本版本接口与最终成品 API 会有很大差异。
// manifest 的 sidebar 只是本实验的挂载点，不是已经确定的 placement API。
import { basename } from "node:path";
import type { PluginContext, PluginDisposer } from "../../sdk/types.ts";

export async function activate(ctx: PluginContext): Promise<PluginDisposer> {
  const paths: string[] = [];
  const pageSize = 3; // 仅为避免在这个实验中引入数组模型桥接；条目总数不受限。
  let page = 0;
  let currentPath: string | null = null;
  let visible = true;
  function setVisible(value: boolean) {
    visible = value;
    return ctx.ui.patch({ visible });
  }

  function render() {
    const start = page * pageSize;
    const count = paths.length;
    const pageCount = Math.max(1, Math.ceil(count / pageSize));
    const items = paths.slice(start, start + pageSize);
    return ctx.ui.patch({
      "items-text":
        items
          .map(
            (path, index) =>
              `${path === currentPath ? "▶" : " "}${start + index + 1}. ${basename(path)}`,
          )
          .join("\n") || "点击“添加文件”建立播放列表",
      status: `共 ${count} 个文件 · 第 ${page + 1} / ${pageCount} 页`,
      count,
      page,
      "has-previous": page > 0,
      "has-next": page + 1 < pageCount,
    });
  }

  const subscriptions = [
    ctx.onAction("add", async () => {
      const path = await ctx.dialogs.openMedia();
      if (path === null) return;
      if (!paths.includes(path)) paths.push(path);
      page = Math.floor(paths.indexOf(path) / pageSize);
      await render();
    }),
    ctx.onAction("play-index", async (index) => {
      if (!Number.isInteger(index) || index < 0 || index >= paths.length)
        return;
      await ctx.player.load(paths[index]);
      // 当前播放标记由后端的 playback 消息决定，不根据成功提交请求猜测。
    }),
    ctx.onAction("previous-page", async () => {
      page = Math.max(0, page - 1);
      await render();
    }),
    ctx.onAction("next-page", async () => {
      page = Math.min(
        Math.max(0, Math.ceil(paths.length / pageSize) - 1),
        page + 1,
      );
      await render();
    }),
    ctx.onAction("hide", () => setVisible(false)),
    ctx.onAction("show", () => setVisible(true)),
    ctx.onAction("toggle", () => setVisible(!visible)),
    ctx.onPlayback(async (state) => {
      if (state.path === currentPath) return;
      currentPath = state.path;
      await render();
    }),
  ];
  await ctx.ui.patch({ visible: true });
  await render();
  // 隐藏侧栏只改变可见性；路径和当前页保留在本插件内存中。
  return () => {
    for (const unsubscribe of subscriptions) unsubscribe();
  };
}
