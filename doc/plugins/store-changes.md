# 给插件维护者：插件商城核心改动（S4）

日期：2026-10-21。播放器 0.2.0、SDK `@vestra-player/sdk` 1.1.0。设计见 [S4 提案](../architecture/store-api-proposal.md)，接口的精确定义见 [API 参考](reference.md)（§2、§3.4、§6.1、§12、§13、§15）。

**结论**：已有接口全部照旧，现有插件不改也能构建、测试、运行（19 个插件 552 项测试不变）。需要动手的只有下表前两行；其余是可选的新能力或发布前的整理。

| 插件 | 必须做 | 建议做 |
| --- | --- | --- |
| 插件管理（`plugins`） | 数据目录改读 `info.dataDirectory`（旧的 `dirname(plugin.dataDirectory)` + id 推算已失效）；`info.builtin` 为真时禁用「卸载…」 | 详情显示来源（`info.source`）；商城（S5）用 `validateArchive`、`install({ source, replace })`、Markdown 组件 |
| 插件工坊（`workshop`） | `install` 处理新的失败：`conflict`（同 id 已从别的来源安装）与 `unsupported`（与内置插件同 id） | 安装时传 `{ source: { kind: "workshop" } }` |
| youtube、bilibili | 无代码改动。密钥改按安装实例存放，已有登录失效一次 | 迁到插件仓库时补显示字段，并写 `"minPlayerVersion": "0.2.0"` |
| 全部插件 | 无 | 补显示字段；把 README 改成面向用户；**改了代码就升 `version`**（内置插件只在版本更高时才会在用户机器上升级） |

## 1. 发生了什么变化

| 变化 | 对插件的影响 |
| --- | --- |
| 数据目录按**安装实例**：`plugin-data/<GUID>/`（开发加载 `plugin-data/development/<id>/`） | 插件自己仍用 `plugin.dataDirectory`，代码不变。开发期的旧数据（`plugin-data/<id>/`）不再读取，不迁移 |
| `secrets` 也按安装实例隔离 | 代码不变；已保存的密钥（登录）要重新保存一次 |
| `packages.list()` 每项多了 `builtin`、`source`、`dataDirectory` | 只读新增；旧字段不变，`origin` 没有新值 |
| 新增 `packages.validateArchive(file)`、`candidate.install({ source, replace })` | 不带参数的 `install()` 行为不变（来源记为「本地文件」） |
| 内置插件：`src/plugins/` 下的插件随安装包分发（来源 `bundled`），只能停用 | `uninstall` 与 `install` 对它们返回 `unsupported` |
| manifest 可选显示字段与 `minPlayerVersion`；未知字段仍拒绝 | 不写不影响；写了就按规则校验（`vestra-plugin check` 会报错） |
| `vestra-plugin build` 会把 `README.md`、`CHANGELOG.md` 与 manifest 的 `icon`/`readme`/`changelog` 打进 `dist/` | README 会出现在商城详情页 |
| `vestra-plugin pack` 产出 `<id>-<version>.vxp`（原 `.zip`，内容相同） | 只影响打包脚本 |
| 新控件库文件 `@vestra/markdown.slint` 与 `plugin.ui.markdown()` | 可用于帮助页、更新说明（见 §4） |

## 2. 插件管理：数据目录与内置插件

```ts
// 之前（已失效）：
const root = dirname(plugin.dataDirectory);
const directoryOf = (id: string) => join(root, id);

// 之后：
const directoryOf = (info: PluginInfo) => info.dataDirectory; // null：播放器没有提供
```

```ts
canUninstall: canAct && !self && installed && !info.builtin,
uninstallTip: info?.builtin ? "内置插件只能停用" : /* 原有提示 */ "",
```

`uninstall` 仍要处理 `unsupported`（列表可能过时）。显示来源：

```ts
const SOURCE: Record<PackageSource["kind"], string> = {
  repository: "仓库", link: "链接", file: "本地文件", workshop: "插件工坊", bundled: "内置",
};
line("来源", info.source ? SOURCE[info.source.kind] + ("url" in info.source ? `（${info.source.url}）` : "") : "开发加载");
```

## 3. 插件工坊：安装来源与冲突

```ts
try {
  const change = await candidate.install({ source: { kind: "workshop" } });
  // …跟随 change 直到结束（不变）
} catch (error) {
  const code = (error as PluginError).code;
  if (code === "conflict") {
    const answer = await plugin.dialogs.confirm({
      message: `替换已安装的 ${id}？`,
      detail: "它是从别处安装的。替换后新插件使用新的空数据目录；原插件的数据保留，以后从原来源重新安装时接上。",
      confirm: "替换",
      destructive: true,
    });
    if (answer.confirmed)
      await candidate.install({ source: { kind: "workshop" }, replace: true });
  } else if (code === "unsupported") {
    // 与内置插件同 id：请改用别的 id
  } else throw error;
}
```

同一来源再次安装同 id 是升级（沿用数据、可回退），不会冲突。「链接」与「本地文件」算同一来源（手动安装）；工坊单独一类，所以工坊生成的插件与用户手动装的同 id 插件会冲突，这是有意的。

## 4. Markdown 组件

```slint
// ui/help.slint
import { MarkdownView, MarkdownBlock } from "@vestra/markdown.slint";
export component Help inherits Window {
    in property <[MarkdownBlock]> doc;
    callback open-link(string);
    MarkdownView { blocks: root.doc; link-clicked(url) => { root.open-link(url); } }
}
```

```ts
const window = plugin.windows.create("ui/help.slint");
window.on("openLink", (url: string) => plugin.system.openUrl(url));
plugin.commands.add({
  id: "help", path: ["帮助", "使用说明…"],
  run: async () => {
    const text = readFileSync(new URL("HELP.md", import.meta.url), "utf8");
    window.set({ doc: await plugin.ui.markdown(text, { base: fileURLToPath(new URL(".", import.meta.url)) }) });
    await window.show();
  },
});
```

- 不需要能力；用到它的插件写 `"minPlayerVersion": "0.2.0"`。
- 文档文件要进包：`.md` 默认不复制，在插件 `package.json` 里写 `"vestra": { "assets": ["HELP.md"] }`（README/CHANGELOG 自动复制）。
- 完整示例：`src/examples/markdown/`；测试里用 `host.markdown(host.window("help").props.doc)` 读回文本。

## 5. 显示字段（发布到官方仓库前）

```json
{
  "id": "vestra.subtitles",
  "version": "0.4.1",
  "apiVersion": "vestra/1",
  "entry": "main.ts",
  "capabilities": ["…"],
  "name": "字幕",
  "author": "Vestra Player",
  "punchline": "载入外部字幕、选择轨道、调整样式",
  "tags": ["字幕"],
  "icon": "assets/icon.png",
  "license": "GPL-3.0-or-later"
}
```

规则见参考 §2。`vestra-plugin check` 会逐项检查，并对没有 `<所有者>.` 前缀的 id 给出提示（不算失败）；`audio`、`nes` 已在阶段 4 改名为 `vestra.audio`、`vestra.nes`（生态计划 1.5a）。

## 6. 内置插件的版本

打包脚本 `src/scripts/stage-plugins.mjs` 把 `src/plugins/` 下每个插件打成 `.vxp` 放进安装包，播放器启动时播种：用户机器上**只有安装包里的版本比上次播种的更高时才升级**（同版本不同内容不会更新；用户回退后也不会被同一安装包覆盖）。所以改了内置插件的代码，请同时升 manifest 的 `version`。

## 7. 测试工具（SDK 1.1）

- `host.packages` 的条目可以带 `builtin`、`source`、`dataDirectory`（都可省略，已有测试不用改）；`builtin: true` 的条目 `uninstall` 返回 `unsupported`。
- `host.candidates.set(路径, manifest)` 让 `validate` / `validateArchive` 接受一个包；`install` 按播放器的来源规则更新 `host.packages`（同来源升级、异来源 `conflict`、带 `replace` 替换），并记入 `host.packageCalls`（`method: "install"`，带 `source`、`replace`）。
- `host.remoteImages.set(地址, 本地文件)` 提供 Markdown 的远程图片，测试不联网。
