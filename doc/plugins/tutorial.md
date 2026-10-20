# 入门教程：写一个「书签」插件

本教程从零写一个完整插件，逐行解释每个文件。读完后你会知道：插件由哪些文件组成、界面和代码怎么通信、插件怎么测试和运行。

完整代码在 `src/examples/bookmark/`，已在真实播放器中运行通过。精确的接口定义见 [API 参考](reference.md)，更多功能的写法见 [插件示例](examples.md)。

## 1. 这个插件做什么

看视频时记住一个位置，待会儿一键跳回去，像看书时夹一张书签：

1. 右键菜单 → 播放 → 书签…，弹出一个小窗口：
   ```
   ┌──────── 书签 ─────────┐
   │ 没有书签               │
   │ [记录当前位置] [跳到书签] │
   └───────────────────────┘
   ```
2. 播到 42 秒时点「记录当前位置」，文字变成「书签：42.0 秒」。
3. 继续看到 5 分钟，点「跳到书签」，视频回到 42 秒。

按钮在不能用的时候是灰的：没打开视频时不能「记录」（没有位置可记）；没记过书签时不能「跳到」。

## 2. 插件由哪些文件组成

```
src/examples/bookmark/      （你自己的插件放在 src/plugins/<名字>/）
├── manifest.json           插件的身份和需要的权限
├── bookmark.slint          界面：长什么样
├── main.ts                 逻辑：数据和行为
└── tests/bookmark.test.ts  测试
```

三者分工：

| 文件 | 负责 |
| --- | --- |
| `manifest.json` | 我是谁、要用播放器的哪些能力 |
| `.slint` | 界面长什么样；接收数据，报告用户点了什么 |
| `main.ts` | 数据是什么、用户点了之后做什么、界面显示什么 |

## 3. `manifest.json`：身份和能力

```json
{
  "id": "community.bookmark",
  "version": "0.1.0",
  "apiVersion": "vestra.experimental/0.1",
  "entry": "main.ts",
  "capabilities": ["commands", "ui.window", "playback.read", "events", "playback.control"]
}
```

- `id`：插件的唯一名字，建议用 `作者或组织.插件名`。
- `version`：三段数字版本。
- `apiVersion`：固定写 `vestra.experimental/0.1`。
- `entry`：入口文件。
- `capabilities`：要用到的播放器能力。用到哪个模块就声明哪个，漏了的话，相关调用会以 `code: "capability"` 失败。

本插件用到：

| capability | 用来做什么 |
| --- | --- |
| `commands` | 往右键菜单里加「书签…」 |
| `ui.window` | 打开书签窗口 |
| `playback.read` | 读取当前播放位置 |
| `events` | 播放位置变化时实时收到更新（没有它，状态只在启动时读一次） |
| `playback.control` | 跳转 |

完整对照表见 [API 参考 §2](reference.md#2-manifest-与能力)。

## 4. `bookmark.slint`：界面

[Slint](https://slint.dev) 是界面描述语言，作用类似网页的 HTML + CSS。它**只描述界面**，不做业务判断。

```slint
import { Button, VerticalBox, HorizontalBox } from "std-widgets.slint";

export component BookmarkWindow inherits Window {
    title: "书签";
    preferred-width: 320px;

    in property <string> label: "没有书签";
    in property <bool> can-mark;
    in property <bool> can-jump;

    callback mark();
    callback jump();

    VerticalBox {
        Text { text: root.label; }
        HorizontalBox {
            Button { text: "记录当前位置"; enabled: root.can-mark; clicked => { root.mark(); } }
            Button { text: "跳到书签"; enabled: root.can-jump; clicked => { root.jump(); } }
        }
    }
}
```

### 界面和插件之间只有两个方向

| 方向 | Slint 写法 | 意思 |
| --- | --- | --- |
| 插件 → 界面 | `in property` | 插件把数据塞进来，界面负责显示 |
| 界面 → 插件 | `callback` | 用户做了什么，界面通知插件 |

逐行看：

- `export component BookmarkWindow inherits Window`：定义一个窗口。
- `in property <string> label`：一个由插件提供的文字，冒号后是默认值。
- `in property <bool> can-mark`：「记录」按钮能不能点，由插件决定。
- `callback mark();`：声明一个叫 `mark` 的**信号**。它本身什么也不做，只是一个可以被发出的通知。
- `Text { text: root.label; }`：显示 `label` 的内容。`root` 指这个窗口本身。
- `enabled: root.can-mark`：按钮是否可点，跟着 `can-mark` 走。
- `clicked => { root.mark(); }`：按钮被点击时发出 `mark` 信号。信号经播放器传给插件，**做什么由插件决定**。界面不知道「记录书签」具体怎么实现。

### 两条规则

**一个文件只导出一个组件。** 插件代码只写文件名（`"bookmark.slint"`），播放器自动使用文件里唯一带 `export` 的组件。导出多个会报错。不带 `export` 的辅助组件可以有任意多个，例如音频工作台的 `component Band` 被重复用了 6 次（`src/plugins/audio/ui/workbench.slint`）。

**会被代码修改的输入控件用双向绑定。** 滑块、勾选框、下拉框被用户操作后，单向绑定 `value: root.x` 会断开：之后插件再改 `x`，控件不再跟着动。这类控件写成 `value <=> root.x`（属性要声明为 `in-out property`）。本例只有按钮，不涉及。

### 外观

播放器以深色风格（`fluent-dark`）编译插件界面，默认字体是系统界面字体，所以上面的窗口不写颜色和字体也是深色、中文正常。要和播放器其它界面保持一致（参考播放器的配色与尺寸），从主题取值、用播放器提供的控件：

```slint
import { Theme } from "@vestra/theme.slint";                 // 颜色、字号、控件尺寸
import { Button, Slider, ToolTipArea } from "@vestra/widgets.slint";
// Rectangle { background: Theme.bg-panel; }  Text { color: Theme.text; font-size: Theme.font-body; }
```

编辑器的 Slint 插件需要知道 `@vestra` 在哪：把语言服务的 `libraryPaths` 设为 `{ "vestra": "<仓库>/src/host/ui/vestra" }`。

## 5. `main.ts`：插件的逻辑

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";

export default async function activate(plugin: Plugin) {
  const { player } = plugin;
  let mark: number | null = null;

  const window = plugin.windows.create("bookmark.slint");
  window.bind(() => ({
    label: mark === null ? "没有书签" : `书签：${mark.toFixed(1)} 秒`,
    canMark: player.state.position !== null,
    canJump: mark !== null && player.state.can.seek,
  }));
  window.on("mark", () => (mark = player.state.position));
  window.on("jump", () => player.seek(mark!));

  plugin.commands.add({
    id: "bookmark.open",
    path: ["播放", "书签…"],
    run: () => window.show(),
  });
}
```

### 入口

```ts
import type { Plugin } from "../../sdk/plugin/index.ts";
export default async function activate(plugin: Plugin) {
```

- 插件启动时，播放器调用默认导出的 `activate`，并传入一个 `plugin` 对象。插件能用的所有功能都挂在它上面：`plugin.player`（播放器）、`plugin.windows`（窗口）、`plugin.commands`（菜单）……
- 只能 `import type`：`plugin` 对象由运行环境提供，插件包本身不依赖本仓库的代码。

### 数据

```ts
  const { player } = plugin;        // 简写：之后 player 就是 plugin.player
  let mark: number | null = null;   // 书签在第几秒；null 表示还没记
```

插件状态就是普通变量，不需要 store，也不需要手动通知界面。

### 窗口

```ts
  const window = plugin.windows.create("bookmark.slint");
```

用界面文件准备一个窗口。此时窗口**还没弹出**，调用 `window.show()` 才会显示。`create`、`commands.add` 这类调用返回**句柄**：之后对这个窗口的一切操作都通过它。插件停用时，SDK 自动关闭所有句柄，不需要写清理代码。

### `bind`：声明界面应该显示什么

```ts
  window.bind(() => ({
    label: mark === null ? "没有书签" : `书签：${mark.toFixed(1)} 秒`,
    canMark: player.state.position !== null,
    canJump: mark !== null && player.state.can.seek,
  }));
```

意思是：窗口的这三个属性，应该分别等于这三个式子的结果。

- `label`：没记过书签显示「没有书签」，否则显示秒数。
- `canMark`：有播放位置（打开了视频），「记录」按钮就能点。
- `canJump`：记过书签，并且当前视频支持跳转，「跳到」按钮就能点。

**你只写一次规则，之后不用管什么时候刷新。** 每当有事情发生（用户点了按钮、播放位置变了、视频被关掉），SDK 自动重新执行这个函数，只把**变化了的**值发给界面。比如视频被关掉时 `player.state.position` 变成 `null`，`canMark` 算出来是 `false`，「记录」按钮自动变灰，这件事插件没有写任何代码。

**名字转换**：代码里写 camelCase（`canMark`），SDK 自动转成 Slint 的 kebab-case（`can-mark`），和 Vue 的 props 规则一样。已经是 kebab-case 的名字原样使用。

### `on`：处理用户操作

```ts
  window.on("mark", () => (mark = player.state.position));
  window.on("jump", () => player.seek(mark!));
```

- 收到 `mark` 信号（用户点了「记录」）：把当前位置存进 `mark`。这里没有写「更新文字」：处理完后 SDK 自动重新执行 `bind`，`label` 就变了，「跳到」按钮也亮了。
- 收到 `jump` 信号：让播放器跳到书签位置。`mark!` 表示「这里一定不是 null」，因为按钮只有在 `mark !== null` 时才能点。
- 回调名同样可以写 camelCase（Slint 的 `toggle-effect` 可写 `"toggleEffect"`）。
- 窗口的回调必须在第一次 `show()` 之前用 `on` 绑定。
- 处理函数可以是 `async`；抛出的错误交给 `plugin.onError`（见 §8）。

### 菜单入口

```ts
  plugin.commands.add({ id: "bookmark.open", path: ["播放", "书签…"], order: 270, run: () => window.show() });
```

- `id`：命令在本插件内的名字。
- `path`：在右键菜单中的位置，最后一项是菜单项文字。同名的上级分组会和其他插件合并。只写一段就是顶层的直接命令。
- `order`：同一级里的位置，小的在前；子菜单「播放」排在它里面最靠前的命令的位置。顶层各段的编号见 [doc/product/menus.md](../product/menus.md)。
- `run`：被点击时执行，这里是弹出窗口。
- 想加快捷键，写 `bindings: ["Ctrl+B"]`；菜单项右侧会自动显示「Ctrl+B」。语法和冲突规则见 [API 参考 §4.1](reference.md#41-绑定)。

## 6. 一次点击的完整过程

视频正在播放，当前在第 42 秒：

```
1. 用户点「记录当前位置」
      ↓
2. Slint：clicked => { root.mark(); }，发出 mark 信号
      ↓（播放器转发给插件）
3. 插件：window.on("mark", …) 被调用，mark = 42
      ↓（SDK 自动）
4. 重新执行 bind 的函数：
      label   = "书签：42.0 秒"   ← 变了
      canMark = true              ← 没变
      canJump = true              ← 变了
      ↓
5. 只把 label、can-jump 发给界面
      ↓
6. 窗口文字更新，「跳到书签」按钮亮起
```

之后点「跳到书签」：Slint 发出 `jump` → 插件调用 `player.seek(42)` → 视频跳回 42 秒。

## 7. 测试：不用启动播放器

`TestHost` 是一个内存里的假播放器，SDK 的真实逻辑照常运行。

```ts
import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "../../../sdk/plugin/testing.ts";
import activate from "../main.ts";

test("mark the position, then jump back to it", async () => {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");                  // 假装用户打开了一个视频
  host.setSource({ duration: 600, position: 42 });   // 时长 600 秒，当前 42 秒
  await startPlugin(activate, { directory: new URL("..", import.meta.url), host });

  await host.command("bookmark.open").run();         // 模拟点菜单
  const window = host.window("bookmark");            // 窗口名默认取文件名
  window.emit("mark");                               // 模拟点「记录」
  await host.settle();                               // 等 SDK 处理完
  assert.equal(window.props.label, "书签：42.0 秒");

  host.setSource({ position: 300 });                 // 视频继续播到 300 秒
  window.emit("jump");                               // 模拟点「跳到」
  await host.settle();
  assert.equal(host.source.position, 42);            // 真的跳回去了
});
```

常用的测试操作：

| 想做的 | 写法 |
| --- | --- |
| 点菜单命令 | `await host.command(id).run()` |
| 模拟界面回调 | `host.window(名字).emit(回调名, ...参数)` |
| 读界面属性 | `host.window(名字).props["can-jump"]`（kebab-case） |
| 模拟视频状态 | `host.openElsewhere(path)`、`host.setSource({ phase, position, duration })` |
| 模拟文件对话框的选择 | `host.dialogAnswers.push(["/a.mp4"])`（取消用 `null`） |
| 模拟播放器拒绝请求 | `host.handle(方法, () => { throw new HostError(code, message) })` |
| 等处理完 | `await host.settle()` |

## 8. 错误处理

处理函数里抛出的错误（包括 async 函数的 reject）由 SDK 统一交给 `plugin.onError`，不需要到处写 `try/catch`：

```ts
let problem = "";
plugin.onError((error) => (problem = `操作失败：${error.message}`));
window.bind(() => ({ label: problem || "…" }));
```

每个错误都有稳定的 `error.code`，例如 `stale`（状态已被别人改变，什么都没做）、`capability`（manifest 没声明能力）。按 `code` 判断，不要按 `message` 文本判断。完整列表见 [API 参考 §13](reference.md#13-错误)。

## 9. 检查、测试、运行、打包

在 `src/` 下执行（自己的插件在 `src/plugins/<名字>/`，命令里直接写名字；示例目录写路径）：

```
npm run plugin -- check examples/bookmark   # 严格类型检查
npm run plugin -- test  examples/bookmark   # 运行 tests/ 下的测试
```

在真实播放器里试用（开发模式直接运行 `main.ts`，不需要先构建；需要 Node ≥ 22.18）。在 `src/host/` 下执行：

```
cargo run --bin vestra-host -- --plugin ../plugins/playback --plugin ../examples/bookmark
```

`--plugin` 可以写多次。这里同时加载基础播放插件，才有「打开」按钮可以打开视频。

打包成可安装的版本：

```
npm run plugin -- build examples/bookmark   # 生成 dist/，安装的就是这个目录
```

- 插件有 npm 依赖时，在插件自己的 `package.json` 里加 `"vestra": { "bundle": true }`，并把 `esbuild` 加入它的 devDependencies，否则安装后会找不到依赖包。
- 构建工具的完整选项见 [API 参考 §15](reference.md#15-构建工具)。

## 10. 下一步

| 想做的事 | 用什么 | 示例 |
| --- | --- | --- |
| 打开文件、播放控制 | `player.open`、`setPaused`、`seek`、`setOutput` | [示例 2、3](examples.md) |
| 挂在底栏或侧栏 | `panels.mount("bottom" \| "sidebar", file)` | [示例 2、4](examples.md) |
| 视频/音频效果 | `video.filter(id)`、`audio.filter(id)` | [示例 1、8](examples.md) |
| 播完自动下一首 | `media.onEnd` + `ifCurrent` | [示例 4](examples.md) |
| 截图、导出片段 | `media.capture` → `snapshot.export` | [示例 5](examples.md) |
| 字幕 | `subtitles.load/select/setStyle` | [示例 7](examples.md) |
| 网络视频解析 | `resolvers.add` | [示例 9](examples.md) |
| 自己生成画面和声音（模拟器） | `player.openStream` | [示例 6](examples.md) |
| 把大插件拆成模块 | `plugin.scope()` | [示例 11](examples.md) |
| 菜单分组、单选、快捷键 | `commands.add` 的 `group`、`radio`、`bindings` | [示例 13](examples.md) |
| 画面上的提示、信息层 | `osd.show`、`osd.panel` | [示例 14](examples.md) |
| 下一个/上一个 | `player.provideSequence`、`player.next` | [示例 15](examples.md) |
| 开关或分离侧栏、面板右键菜单 | `panels.sidebar`、`panel.showMenu` | [示例 16](examples.md) |
| 深色主题与参考播放器样式控件 | `@vestra/theme.slint`、`@vestra/widgets.slint` | [示例 17](examples.md) |

两个常见坑：

- **在自己的计时器里改了状态**：SDK 只在它调用的处理函数之后自动刷新界面。用 `setInterval`、`setTimeout` 或长任务中途改了变量时，调用一次 `plugin.update()`。
- **操作以你看到的状态为条件**：`seek`、`setPaused` 这类控制默认以 `player.state` 为准。如果在你看到之后，别的插件或用户换了视频，调用以 `code: "stale"` 失败、什么都不做，不会误操作到别的视频上。需要以更早的状态为条件时传 `{ ifCurrent: state }`，见 [API 参考 §5.3](reference.md#53-条件提交ifcurrent)。
