# 面板槽位共存（侧栏页签）

2026-10-24。用户裁决：**方案 A，侧栏页签**。底栏和 backdrop 保持独占。

## 问题

槽位只有 3 个（`Slot::Bottom`、`Slot::Sidebar`、`Slot::Backdrop`，`composition.rs`），每个槽位只能放一个面板：

- `shell_source` 遇到两个面板同槽就 bail；
- `window_state::Policy` 的 `bottom`/`sidebar` 是 `Option<Attachment>`；
- `InputShared.sidebar` 只记一个 owner；
- `layout.set`、`sync_floating`、`floating_source` 都按「一个槽位一个 owner」写。

侧栏和底栏的冲突要到编译阶段才发现，报的是 `ui-prepare`；而 `activate` 期间挂载失败会让整个激活失败。结果是后挂的插件（比如内置播放列表）整体失效，「播放列表 F6」消失，用户看不到任何提示。

## 第 0 步：失败必须看得见（分支 `fix/slot-conflict`）

- `ui.mount` 入口就检查侧栏和底栏是否已被占用（包括排队中、准备中的挂载），占用时返回 `conflict`，消息里写明占用方的插件 id。
- 槽位冲突不再让激活失败：插件照常 ready，带上 `slot-occupied` 警告，插件管理里显示出来。
- 内置插件先挂。
- `ui-prepare` 只用于真正的编译错误和桥接错误。

## 第 1 步：侧栏页签

```
┌──────────── video ────────────┬[播放列表][示例列表]   ⧉ ✕┐
│                               │  1. a.mkv                 │
└──────────── bottom bar ───────┴───────────────────────────┘
```

- **页签条**：只有一个侧栏面板时不画，界面和现在完全一样；两个及以上时由宿主画 31px 页签条，样式沿用 ui-spec §5 的播放列表页签。分离和关闭按钮放在页签条右端，作用于整个槽位。
- **宿主**
  - composition：每个侧栏面板生成 `if root.sidebar-active == N: Owner{N}View`，切换页签只改属性，不重编 shell。
  - `floating_source` 改为接收多个 spec，浮动窗口带同样的页签条。
  - `Policy.sidebar` 改为 `{ panels, active, preferred, detached }`：可见和分离是槽位级的。
  - `InputShared.sidebar` 取当前页签的 owner，只有当前页签收到 panel 键。
  - `layout.set` 增加 `active`。
  - `ui_state` 增加 `sidebarSlot { visible, detached, active, order }`；旧的 `panels[*]` 迁移过来。
- **SDK**：只做增量（`vestra/1`，patch 版本）。
  - `LayoutSlot.owner` 的含义改为当前页签的 owner。
  - 新增 `LayoutSlot.panels?`，`reason` 增加 `"otherTab"`。
  - `Panel` 新增 `state` / `onChange` / `show()` / `toggle()`；`PanelSlot` 新增 `activate(plugin)`。
  - 第二个侧栏面板的 `ready` 照常 resolve；`showMenu` 在后台页签上调用时拒绝，返回 `unavailable`。
  - 老插件不用改。
- **播放列表**
  - F6 改为 `list.toggle()`，勾选状态 = 侧栏可见且当前页签是播放列表。
  - 有宿主页签条时，隐藏自己的页签条。
  - 遇到旧宿主时回退到 `sidebar.toggle()`。
- **快捷键**：插件可以给自己的 `toggle()` 绑键；可选 Ctrl+Tab（限定 `focus:"panel"`）切换页签，加之前先查 menus.md 有没有冲突。
- **分离、全屏、拖放**：分离的是整个侧栏，浮动窗口里也有页签；全屏弹出层显示当前页签；拖放交给当前页签。
- **首版不做**：关闭单个页签、拖动排序。页签按安装顺序排，播放列表固定在第一个。

## 抢不到槽位时插件拿到什么

不能静默失败：

- 登记了但当前不显示：`ready` resolve，`panel.state.active === false`，`reason` 为 `"otherTab"`，状态变化通过 `onChange` 通知。
- 独占槽位被占：`ready` reject，返回 `conflict`，消息里有占用方的插件 id。
- 两种情况都不会让激活失败；插件管理里有提示。

## 测试

- 单测：composition 生成多个分支并能编译；window_state 中当前页签的回退；ui_state 迁移；`ui.mount` 返回 `conflict`、`layout.set` 的 `active`；SDK fake host；播放列表的 F6。
- 真机：同时加载 panel-menu 示例和播放列表，检查以下操作：
  - 页签切换；
  - F6；
  - 焦点在列表上时按 F6 关闭；
  - 分离 / 附着；
  - 全屏弹出层；
  - 拖放；
  - 重启后恢复当前页签；
  - 窄窗口。
- 完成后删掉 panel-menu README 里「不要和播放列表同时加载」那句。

不新增系统相关代码：浮动窗口沿用现有的 `platform::` 调用。
