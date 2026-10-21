# S4 核心 API 变更提案：安装实例、来源、内置保护、manifest 字段、Markdown 组件

日期：2026-10-21。作者：API 维护者。状态：**已确认（2026-10-21）**，见文末「协调者确认」。
依据：[生态计划](../product/ecosystem-plan.md) 第 1 章（1.2、1.3、1.4、1.5a、1.6）与第 4 章 S4；[工作流](../process/community-workflow.md) §1、§1.5、§1.6、§4.3。

## 0. 结论

| # | 改动 | 位置 | 对插件 API |
| --- | --- | --- | --- |
| 1 | 安装登记并入现有 `packages/catalog.json`：每个 id 的当前安装带一个**安装实例**（GUID、来源、内置标记、启用、安装时间）；被替换或卸载（保留数据）的实例转为「休眠」，换回同一来源时接上 | 宿主 | 无变化（插件仍从 `VESTRA_PLUGIN_DATA_DIR` 取目录） |
| 2 | 数据目录 `plugin-data/<id>/` → `plugin-data/<GUID>/`；开发加载 `plugin-data/development/<id>/`；`secrets` 也按 GUID 隔离 | 宿主 | **文档契约变化一处**：§3.4「`<数据根>/<id>` 推出别的插件的数据目录」作废，改用新字段 `PluginInfo.dataDirectory`（§7.3） |
| 3 | `packages` 新增：`validateArchive(file)`（`.vxp`）、`install({ source, replace })`、`PluginInfo` 加 `builtin`、`source`、`dataDirectory` | 宿主 + SDK | 只新增 |
| 4 | 内置插件按**来源**识别（随安装包播种），`uninstall` → `unsupported`，跨来源替换 → `unsupported`；停用、回退照常 | 宿主 | 只新增错误情形 |
| 5 | manifest 新增可选显示字段与 `minPlayerVersion`，宿主与 CLI 用同一张规则表校验；未知字段**继续拒绝** | 宿主 + SDK CLI | 只新增 |
| 6 | 核心 Markdown 组件：`@vestra/markdown.slint` 的 `MarkdownView` + `plugin.ui.markdown()` 生成的属性值；宿主用 pulldown-cmark 解析，远程图片由 SDK 在插件进程里下载缓存 | 宿主 + SDK + 控件库 | 只新增，无新能力 |
| 7 | SDK 1.1.0：类型、TestHost、CLI（新字段校验、无前缀 id 提示、build 带上 icon/README、`pack` 产出 `.vxp`） | SDK 仓库 | 只新增；`pack` 文件扩展名变化（非插件 API） |
| 8 | 打包：`Resources/plugins/` 放内置 `.vxp` 与 `builtin.json`，宿主启动时播种 | 宿主 + `src/scripts/` | 无 |

不进核心：下载 `index.json`、签名校验、更新检查、撤回处理、仓库管理、权限展示——都在商城插件（S5）。

---

## 1. 安装实例 GUID 与安装登记（1.5a）

### 1.1 现状

| 代码 | 现在按什么 | 问题 |
| --- | --- | --- |
| `package_store.rs` `Catalog.installed: BTreeMap<id, Installed>`、`disabled: BTreeSet<id>` | id | 没有来源、没有实例；停用集合单独存 |
| `plugins.rs` `load_manifest`：`data_root().join(&manifest.id)` | id | 同 id 的两个插件共用数据目录 |
| `package_manager.rs` `uninstall`：`plugins::data_root().join(&id)` | id | 同上 |
| `system.rs` `secrets.*`：keychain service = manifest id | id | 替换后的同 id 插件能读到前一个插件的密钥 |
| `main.rs` `load`（`--plugin`）：数据目录同样是 `<id>` | id | 开发加载与已安装的同 id 插件共用数据 |

### 1.2 登记的位置与格式

**不另建文件**，扩展现有的 `<app_data_dir>/packages/catalog.json`（`VESTRA_PACKAGE_STORE` 可改根目录）。理由：实例必须和安装、回退、恢复在**同一次原子替换**里变化（`Store::publish` 已是单写者 + 临时文件改名）；另开一个文件就要处理两份文件不一致。

```jsonc
{
  "installed": {
    "vestra.youtube": {
      "revision": 3,
      "instance": {                          // 新增：当前安装实例
        "guid": "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41",
        "source": { "kind": "repository", "url": "https://…/index.json" },
        "builtin": true,
        "enabled": true,                     // 取代原 disabled 集合
        "installedAt": 1792540800000,        // 该实例首次安装，Unix 毫秒
        "seededVersion": "1.2.0"             // 仅内置：上次随安装包播种的版本（§3.3）
      },
      "current": { "digest": "…", "manifest": { … }, "directory": "…/packages/<digest>" },
      "previous": { … }                      // 同一实例内的上一版本，回退用
    }
  },
  "dormant": {                               // 新增：休眠实例（只剩数据）
    "9b1d…": { "id": "someone.viz", "source": { "kind": "link", "url": "…" },
               "version": "0.3.0", "installedAt": …, "retiredAt": … }
  },
  "pending": { … }, "operations": { … }, "lastCommit": { … }
}
```

| 字段 | 说明 |
| --- | --- |
| `guid` | 宿主生成的 UUID v4，小写带连字符。全新安装时生成；升级、回退沿用 |
| `id`、版本 | 分别是 `installed` 的键与 `current.manifest.version`，不重复存 |
| `source` | §2.3 的 `PackageSource`；内置插件的来源是官方仓库地址，由 `builtin.json` 给出 |
| `builtin` | 由宿主播种时写入，插件无法设置 |
| `enabled` | `setEnabled` 写这里；原 `disabled` 集合删除（读到旧 catalog 时按集合换算一次） |
| `installedAt` | 实例首次安装时间 |

上限：`dormant` 至多 256 条；满了以后新的替换/卸载照常进行，只是**不再保留可接上的登记**（数据目录仍留在磁盘，不自动删除用户数据），并在宿主日志里记一行。

未发版，不迁移：旧 catalog 里没有 `instance` 的条目，打开时补一个新 GUID（数据目录为空），旧的 `plugin-data/<id>/` 不动也不读。

### 1.3 规则

| 情形 | 行为 |
| --- | --- |
| 全新安装 id | 新 GUID，空数据目录 |
| 同 id、**同来源**（§2.3 的来源标识相同） | 升级：沿用 GUID、数据、`enabled`、`builtin` |
| 同 id、**不同来源** | 未带 `replace: true` → `conflict`；带了 → 替换：当前实例转休眠（包文件回收，数据保留），新实例拿新 GUID 与空目录 |
| 替换时，新来源恰好对应同 id 的一个休眠实例 | 「换回」：复用那个 GUID 与数据，休眠记录删除 |
| 替换的目标是内置插件 | `unsupported`（内置不可替换），`replace` 也不行 |
| 替换激活失败、取消、播放器中途退出 | 现有恢复流程把 `Installed`（含 `instance`）整体还原；新实例这次创建的数据目录删除 |
| `rollback` | 只在同一实例内（`previous`），GUID 不变。替换后的新实例没有 `previous` |
| `uninstall`，不带 `removeData` | 当前实例转休眠，同来源重新安装时接上数据（等于今天「卸载保留数据、重装恢复」的行为） |
| `uninstall({ removeData: true })` | 删除该实例的登记与数据目录（进程退出后），休眠记录也不留 |
| 同时运行 | 同一 id 只运行一个：`load_package` 现有的 `plugin already running` 检查不变 |

### 1.4 目录与密钥

| 对象 | 现在 | 改为 |
| --- | --- | --- |
| 已安装插件数据目录 | `plugin-data/<id>/` | `plugin-data/<GUID>/` |
| 开发加载（`--plugin`） | `plugin-data/<id>/` | `plugin-data/development/<id>/`（与已安装的同 id 插件互不影响） |
| 试载 | `plugin-data/<id>.trial.<owner>/`，停止时删除 | 不变 |
| `secrets` 的 keychain service | `<id>` | 已安装：`vestra.plugin.<GUID>`；开发加载：`vestra.dev.<id>`；试载：不变（service 是试载自己的 id `<id>.trial.<owner>`，本来就与正式安装分开）。`removeData: true` 卸载时一并删除该 service 下的条目（做不到的平台返回 `unsupported` 不阻塞卸载，见 §9.3 待验） |
| 界面偏好（侧栏、浮动窗口位置、绑定覆盖层、选项窗口页） | `<id>` | **不变**：它们是用户对该 id 界面的偏好，不是插件私有数据，换实例后沿用合理 |

`VESTRA_PLUGIN_DATA_DIR` 仍是插件拿目录的唯一途径，插件代码不变。

### 1.5 代码改动

| 文件 | 改动 |
| --- | --- |
| `package_store.rs` | `Installed` 加 `instance: Instance`；`Catalog` 加 `dormant`，删 `disabled`（旧字段读入时换算）；`prepare_with_receipt` 接收目标实例（升级沿用 / 新建 / 换回）；`commit` 时写入；`uninstall(id, keep_data)` 转休眠或删除；`set_enabled` 改写 `instance.enabled`；`validate_catalog` 校验 GUID 格式、`dormant` 上限、同一 GUID 不同时出现在 `installed` 与 `dormant` |
| `package_manager.rs` | `packages.install` 读 `source`、`replace`，按 §1.3 决定实例并在冲突时返回 `conflict`；`uninstall` 拒绝内置；`load_package` 把实例传给 `Plugin::load_manifest`；`package_list` 输出 `builtin`、`source`、`dataDirectory`；数据删除路径改为 GUID 目录 |
| `plugins.rs` | `load_manifest` 新参数 `data_key: DataKey`（`Instance(guid)` / `Development(id)` / `Trial(owner)`），据此建目录；`Plugin` 记住 `data_key` 供 `secrets` 用 |
| `main.rs` `load` | 开发加载用 `DataKey::Development` |
| `system.rs` | `secrets.*` 的 service 取自 `data_key` |
| `Cargo.toml` | `uuid = { version = "=1.x", default-features = false, features = ["v4"] }`（已在依赖树里，MIT/Apache-2.0） |

---

## 2. `packages` 接口新增（`plugins.manage`）

### 2.1 现有接口（不变）

`validate(directory)`、`rollback(id)`、`list()`、`onChange()`、`setEnabled(id, enabled)`、`uninstall(id, { removeData })`；`PackageCandidate.trial()`、`install()`。调用方：插件仓库中的 `plugins/plugins/`（`list`、`onChange`、`setEnabled`、`rollback`、`uninstall`）、`plugins/workshop/`（`validate`、`trial`、`install`、`rollback`）。

### 2.2 新增签名

```ts
interface Packages {
  // 新增：从 .vxp 文件（zip）校验。返回与 validate 相同的 PackageCandidate
  validateArchive(file: string): Promise<PackageCandidate>;
}
interface PackageCandidate {
  // 原 install() 增加可选参数；不带参数时与现在相同（来源为 { kind: "file" }）
  install(options?: { source?: PackageSource; replace?: boolean }): Promise<PackageChange>;
}
type PackageSource =
  | { kind: "repository"; url: string }   // index.json 的完整地址（商城规范化后传入）
  | { kind: "link"; url: string }         // 插件包链接
  | { kind: "file" }                      // 拖入或选择的本地包、目录
  | { kind: "workshop" };                 // 插件工坊生成
interface PluginInfo {
  // 现有字段不变，新增：
  builtin: boolean;                  // 随播放器安装：可停用，不可卸载、不可被其它来源替换
  source: PackageSource | null;      // origin 为 "installed" 时有；开发加载、试载为 null
  dataDirectory: string | null;      // 它的数据目录；播放器没能提供时 null
}
interface PackageManifest {
  // 现有字段不变，新增可选：name、author、punchline、description、tags、icon、
  // readme、changelog、sourceUrl、license、minPlayerVersion（§4）
}
```

- `validateArchive`：宿主解压到临时目录后走 `Candidate::capture`，检查与 `validate` 完全相同（manifest、digest、`node --check`、Slint 编译）。`.vxp` 的 sha256 由调用方（商城）在调用前自己核对，宿主不知道期望值。
- `install` 的 `source` 进 wire 的 `packages.install` 参数，所以也进入请求指纹：同一 `requestId` 换了来源会以 `conflict` 拒绝（现有去重规则）。
- `PluginInfo.origin` **不加新值**（内置仍是 `"installed"`，用 `builtin` 区分）：给输出类型的联合加成员，会让作者写的穷举 `switch` 编译失败，属于隐性破坏。

### 2.3 来源标识（决定「升级」还是「替换」）

| `kind` | 来源标识 | 理由 |
| --- | --- | --- |
| `repository` | `url`（WHATWG 规范化后逐字比较） | 商城「只从原来源接收更新」（1.5a） |
| `link`、`file` | 同为 `manual` | 用户手动装同 id 的包就是有意更新；按链接区分会让「换一个版本链接」变成替换并丢数据 |
| `workshop` | `workshop` | 工坊生成的 id 与手动安装的插件撞名时应提示 |

`url` 只记录、只比较，宿主不访问它。`url` 须为 `http(s)`、≤ 2048 字节，否则 `invalid`。**这一条需要拍板**（见报告）：若希望链接安装也按 URL 区分，改表即可，接口不变。

### 2.4 替换确认：宿主拒绝，插件确认后带标志重试

选择：同 id 已从**另一来源**安装时，`install()` 以 `code: "conflict"` 失败、什么都不做；插件用 `dialogs.confirm` 问用户，同意后 `install({ source, replace: true })`。

| 方案 | 结论 |
| --- | --- |
| **宿主拒绝 + `replace` 标志**（采用） | 判断在宿主里与提交原子进行（插件先 `list()` 再装，中间状态可能变）；确认界面留在插件（文案、复选框由商城/工坊决定，宿主不替插件弹框，符合「核心不含插件业务」）；不带标志的旧调用不会静默替换 |
| 宿主自己弹确认框 | 宿主要知道「商城」「工坊」的文案和场景；无界面模式无法工作 |
| 只靠插件先 `list()` 自查 | 有竞态；漏查的插件会静默覆盖别人的插件 |

插件在弹框前想显示「已安装的来自哪里」，用 `list()` 里该 id 的 `source`、`version`；错误对象不额外携带数据（保持 `PluginError = Error & { code }` 不变）。

### 2.5 GUID 不暴露

- 插件按 id 寻址，同一时间每个 id 只有一个实例，GUID 没有可调用的接口。
- 插件管理真正需要的只是数据目录，已由 `dataDirectory` 给出。
- 不承诺 GUID，宿主以后可以改存储布局；插件也无法持久化、比对实例（例如据此追踪用户换过哪些来源）。
- `dataDirectory` 的路径里看得到 GUID，但文档写明「路径格式不属于契约」。

### 2.6 回退

不改。`rollback(id)` 回到同一实例的 `previous`；内置插件可以回退（§3.3 保证播种不会把用户的回退立刻覆盖掉）。跨来源「回到旧插件」= 用旧来源重新安装（会接上休眠实例的数据）。

---

## 3. 内置插件：只能停用，不能卸载

### 3.1 识别

按**来源**：只有宿主启动时从安装包的 `Resources/plugins/builtin.json` 播种的实例带 `builtin: true`。插件不能通过 `install()` 声明内置；id 不参与判断（`vestra.` 前缀得不到任何东西，1.5a）。

### 3.2 行为

| 操作 | 内置插件 |
| --- | --- |
| `setEnabled(id, false/true)` | 照常，跨启动保持 |
| `uninstall` | `code: "unsupported"`，message `built-in plugins can be disabled but not uninstalled` |
| `install` 同来源（官方仓库）的新版本 | 升级，保持 `builtin` |
| `install` 其它来源，带不带 `replace` | `unsupported`（内置不可替换） |
| `rollback` | 照常 |

### 3.3 播种（宿主启动时）

`builtin.json`（打包脚本生成）：

```json
{ "schema": 1, "source": { "kind": "repository", "url": "https://…/index.json" },
  "plugins": [ { "file": "vestra.youtube-1.2.0.vxp", "sha256": "…" } ] }
```

| 已安装状态 | 动作 |
| --- | --- |
| 没有这个 id | 按 `.vxp` 安装（新 GUID，`builtin: true`，`seededVersion` = 包版本） |
| 已安装且是内置 | 包版本 > `seededVersion` 才升级到包里的版本（用户经商城装了更高版本则不动）；用户回退过也不会被同一安装包再次覆盖 |
| 已安装但来自其它来源（发版前用户侧载了同 id） | 不动，日志一行；内置标记不强加 |

播种走现有的 stage → prepare → 激活 → commit 流程（失败按现有规则恢复并记入 `failures`），不另写安装路径。`VESTRA_BUILTIN_PLUGINS=<目录>` 可在开发时指定别的播种目录；直接 `cargo run` 没有该目录时不播种。

`--plugin` 开发加载与已安装插件同 id 时：开发加载优先，已安装的这次运行不启动（`state: "stopped"`）。实现时先核对 `package_tick` 与 `load` 的现有顺序。

### 3.4 `plugins` 插件与工坊需要知道的

- 插件管理：`builtin` 为真时禁用「卸载…」，悬停说明「内置插件只能停用」；详情显示来源（官方仓库 / 第三方仓库地址 / 链接 / 本地文件 / 插件工坊）。`uninstall` 仍要处理 `unsupported`（列表可能过时）。
- 工坊：生成的 id 与内置插件撞名时 `install` 得到 `unsupported`，提示用户改 id。

---

## 4. manifest 新字段

### 4.1 字段与校验（宿主与 CLI 同一张表）

| 字段 | 类型 | 规则 |
| --- | --- | --- |
| `name` | string | 1–64 个字符（Unicode 标量），无控制字符，首尾无空白 |
| `author` | string | 1–64 个字符，同上 |
| `punchline` | string | 1–120 个字符，单行 |
| `description` | string | 1–2000 个字符，允许换行，纯文本 |
| `tags` | string[] | 至多 8 个，每个 1–24 个字符、单行，不区分大小写去重 |
| `icon` | string | 包内相对路径（与 `entry` 相同的路径规则：不含 `..`、`\`、绝对路径、符号链接）；扩展名 `.png`/`.jpg`/`.jpeg`；文件必须在包内；≤ 512 KB；按内容判断为 PNG/JPEG，边长 ≤ 1024（建议 256×256 正方形） |
| `readme` | string | 包内相对路径，`.md`，UTF-8，≤ 256 KB。省略时若包里有 `README.md` 则视为它 |
| `changelog` | string | 同 `readme`；省略时若有 `CHANGELOG.md` 则视为它 |
| `sourceUrl` | string | `https:` 地址，WHATWG 解析成功，≤ 2048 字节 |
| `license` | string | 1–64 个字符（建议 SPDX 表达式，不对照 SPDX 列表） |
| `minPlayerVersion` | string | 与 `version` 相同的严格 `x.y.z`（§6） |

- 显式写出的 `icon` / `readme` / `changelog` 找不到文件 → 拒绝；默认的 `README.md` / `CHANGELOG.md` 不存在 → 不报错。
- 字段只用于显示，宿主除了校验不使用它们（`minPlayerVersion` 除外）。宿主不渲染 README，也不显示 icon。
- 开发加载（`--plugin`）同样校验；路径相对源码目录。

### 4.2 未知字段：继续拒绝

现在 `plugins::Manifest` 与 `package_stage::PackageManifest` 都是 `deny_unknown_fields`，参考文档也写明拒绝。保持：

- 拼错的字段（`capabilties`）立刻暴露，不会悄悄失效；
- 以后新增字段时作者同时写 `minPlayerVersion`，商城不会把它推给旧播放器；
- 侧载到旧播放器时，宿主**先读 `minPlayerVersion`** 再做严格解析，所以报的是「需要 Vestra Player ≥ x.y.z」而不是「未知字段」。

实现提示：两个 manifest 结构并成一个模块（建议 `src/host/src/manifest.rs`），一套字段、两种校验模式（开发/包）。serde 的 `flatten` 不能与 `deny_unknown_fields` 同用，所以不要用 `flatten` 复用字段。

### 4.3 id 规则

不变（1.5a 已定）：非空、≤ 128 字节、ASCII 字母数字与 `.-_`。宿主不强制前缀，不给任何 id 特殊待遇。

---

## 5. 核心 Markdown 显示组件（1.6 第 10 条）

### 5.1 形态：控件库新控件 + 宿主解析成模型

```slint
import { MarkdownView } from "@vestra/markdown.slint";
export component Readme inherits Window {
  in property <[MarkdownBlock]> readme;      // 插件写入 plugin.ui.markdown() 的结果
  callback open-link(string);
  MarkdownView { blocks: root.readme; link-clicked(url) => { root.open-link(url); } }
}
```

```ts
const doc = await plugin.ui.markdown(text, { base: readmeUrl });   // 不需要能力
window.set({ readme: doc });
window.on("openLink", (url: string) => plugin.system.openUrl(url));
```

与现有架构的对应：

| 环节 | 做法 | 依据 |
| --- | --- | --- |
| 线上传值 | `MarkdownRef` 与 `ImageRef` 同类：只能作为属性的整个值，线上形式 `{ "$vestraMarkdown": { text, base, images }, "ref": n }` 是内部细节 | `ui_image.rs` 的 `$vestraImage`；`Host::prop_value` 加一个分支 |
| 解析 | 宿主在 `prop_value` 里解析成 `[MarkdownBlock]` 的 Slint 模型（`slint_interpreter::Value::Model` of Struct），图片直接构造成 `Value::Image` | 图片不能经 JSON 进结构体（参考 §6.1 的限制），宿主侧构造模型绕开这一点 |
| 图片加载 | 复用 `ui_image::Images`：同样的路径根（插件包与数据目录）、格式、16 MB / 4096² 上限、每插件 64 MB 缓存 | `ui_image.rs` |
| 渲染 | `src/host/ui/vestra/markdown.slint`（新库文件）导出 `MarkdownView` 与 `struct MarkdownBlock`；`ScrollView` + `for block in blocks` 按 `kind` 画标题、段落、列表项、代码块、引用、图片、表格、分隔线 | 控件库机制不变；`sync-sdk-slint.mjs` 的 `SLINT_FILES` 加入新文件 |
| 链接 | `MarkdownView` 的 `callback link-clicked(string)` 给出绝对地址（相对链接按 `base` 解析；`#锚点` 原样给出），打开与否由插件决定（通常 `system.openUrl`） | 已有 `system.openUrl` |

放在单独的库文件而不是 `widgets.slint`：控件较大，不用它的插件编译时不加载。

### 5.2 支持的子集

| 元素 | 渲染 |
| --- | --- |
| 标题 1–6 | 字号按 `Theme` 分级 |
| 段落、强调、加粗、删除线、行内代码、链接 | 见 §5.4 |
| 无序 / 有序列表、任务列表 | 嵌套深度 ≤ 4（更深的压平到第 4 层），任务项显示 ☐ / ☑ |
| 代码块 | `Theme.font-mono`，自动换行，不做语法高亮 |
| 引用 | 左侧竖线，嵌套 ≤ 4 |
| 图片 | 本地或已下载的 PNG/JPEG；按宽度缩放不放大；失败、不支持的格式（SVG、GIF、WebP）显示 alt 文字 |
| 表格（GFM） | 单元格为纯文本（去掉行内样式），列宽均分，≤ 16 列 × 500 行 |
| 分隔线 | 有 |
| 内嵌 HTML（块与行内） | **丢弃**，不显示原文 |
| 脚注、数学、定义列表、front matter | 不启用，按普通文本 |

上限：`text` ≤ 256 KB（线上消息 1 MiB），块 ≤ 4000，图片 ≤ 64；超出部分截断并在末尾加一块「内容过长，已截断」。

### 5.3 远程图片：SDK 在插件进程下载，缓存在插件数据目录

| 方案 | 结论 |
| --- | --- |
| **SDK 下载**（采用） | Node 自带 `fetch`，跨平台、无新依赖；下载属于调用插件，资源与停用一起回收；缓存在该插件自己的数据目录，`ui_image` 的路径根不用放宽 |
| 宿主下载 | Rust 侧要新增 HTTP + TLS 栈（ureq/rustls 与各平台证书），扩大核心与跨平台面，只为一个显示功能 |

流程：`ui.markdown(text, { base })` → SDK 发 `ui.markdownScan`（宿主解析，返回需要的远程图片绝对地址，至多 64 个）→ SDK 下载到 `<dataDirectory>/.vestra-cache/markdown/<sha256(url)>` → 构造线上值，`images` 为「地址 → 本地路径」。

| 规则 | 值 |
| --- | --- |
| 协议 | 只 `https:`；`http:` 与其它显示 alt |
| 单张 | ≤ 4 MB，按内容只收 PNG/JPEG，10 秒超时，重定向 ≤ 5 次，不带 cookie |
| 并发 | 每插件 4 |
| 缓存 | 每插件 32 MB，按最近使用淘汰；同一地址 24 小时内不重新下载 |
| 失败 | 不让 `markdown()` 失败，对应图片显示 alt |
| `options.remoteImages: false` | 不下载 |

### 5.4 行内样式：用 Slint 的 `StyledText`，需要先验证（待拍板）

Slint 1.16.1 已有 `StyledText` 元素（加粗、斜体、删除线、行内代码、链接与 `link-clicked`），但 `slint-interpreter` 里对应的 `Value::StyledText` 标着 `#[doc(hidden)]`（「StyledText 公开后再显示」），宿主要构造它得用未公开的接口。

- 实现第一步做探针 M0：在插件窗口里用解释器显示宿主构造的 `StyledText`，含中文、链接点击。
- 通过：段落、列表项、引用、标题用 `StyledText`；构造代码集中在一个函数里，Slint 锁定 `=1.16.1`，升级 Slint 时必须复测。
- 不通过：降级为纯文本（去掉行内标记，行内代码保留文字），段落里的链接列在该段下方作为可点击行。接口不变，以后再升级渲染。

### 5.5 Rust 解析库

`pulldown-cmark`：MIT 许可，CommonMark + GFM 表格/删除线/任务列表；**已在依赖树里**（Slint 1.16.1 依赖 0.13.4），直接依赖锁同一版本 `=0.13.4`、`default-features = false`，不引入新代码。第一方代码仍 `forbid(unsafe_code)`。解析是纯计算，与系统无关，满足 §1.5 跨平台规则。

### 5.6 能力

不新增能力。与 `ui.image` 相同：只是一种属性值，显示它要的 `ui.window` / `ui.mount` / `ui.settings` 已经限制了能显示的地方；下载走插件自己的进程，能力本来也不限制联网。

---

## 6. `minPlayerVersion`

- **播放器版本号来源**：宿主 `Cargo.toml` 的 `version`（`env!("CARGO_PKG_VERSION")`），即插件看到的 `plugin.app.version`。打包时 `Info.plist` / Windows 版本资源从它生成，不另维护一份。建议 S4 发布时把宿主从 `0.1.0` 升到 `0.2.0`，作为第一个支持新 API 的版本。
- **语义**：播放器版本（只比较 `MAJOR.MINOR.PATCH` 三段数字，忽略预发布后缀）≥ `minPlayerVersion` 才能加载。它表达「用到了某版播放器才有的**新增** API」；`apiVersion` 仍表达破坏性的大版本。没有 `maxPlayerVersion`。
- **检查点**：`validate` / `validateArchive` → `code: "versionMismatch"`（宿主已有此码，补进参考 §13），message `requires Vestra Player ≥ x.y.z`；开发加载 → 加载失败同样的消息；已安装的插件在降级后的播放器上 → 不启动，`state: "failed"`、`error` 同上；内置播种跳过并记日志。
- **SDK 版本对照**：SDK README 加一张表「SDK 1.1 新增的 API 需要播放器 ≥ 0.2.0」。CLI 不自动推断（要分析用到了哪些 API，不值得）。
- `index.json` 的每个版本条目建议加 `minPlayerVersion`，商城据此过滤；这属于 S3/S5，见 §8。

---

## 7. SDK 1.1.0 范围

### 7.1 内容

| 部分 | 内容 |
| --- | --- |
| 类型（`types.ts`） | §2.2 全部；`PackageManifest` 新增可选字段；`plugin.ui.markdown(text, { base?, remoteImages? }): Promise<MarkdownRef>`、`MarkdownRef` 类型；`PackageSource` |
| 实现 | `packages.ts`：`validateArchive`、`install(options)`、`info()` 带出新字段；`ui.ts`：`markdown()` 与下载缓存（只用 Node 内置模块，保持零运行时依赖） |
| wire（内部） | `packages.validateArchive`；`packages.install` 参数加 `source`、`replace`；`packages.list` / `packages.changed` 条目加字段；`ui.markdownScan` |
| TestHost | `host.packages` 条目的新字段**全部可选**（默认 `builtin: false`、`source: null`、`dataDirectory: null`），已有测试不用改；`uninstall` 内置条目 → `unsupported`；新增 `host.candidates`（路径 → manifest）支持 `validate` / `validateArchive` / `install`，按 §1.3 模拟 `conflict` 与 `replace`，`packageCalls` 记录 `install`；`ui.markdown` 的线上值可用 `host.markdown(value)` 读回 `{ text, base, images }`；远程图片不联网，由 `host.remoteImages`（地址 → 本地路径或 `null`）提供 |
| CLI `check` | 按 §4.1 校验全部新字段（规则表从 `./runtime` 导出常量，宿主一致性测试比对）；id 不含 `.` 前缀时打印**提示**（如「发布到官方仓库需要 `<GitHub 用户名>.<插件名>`」），退出码 0；manifest 未知字段报错（与宿主一致） |
| CLI `build` | 把 manifest 引用的 `icon`、`readme`、`changelog`（及默认的 `README.md`、`CHANGELOG.md`）复制进 `dist/`——现在 `.md` 一律跳过，不改的话 README 进不了包 |
| CLI `pack` | 产出 `<id>-<version>.vxp`（内容与现在的确定性 zip 相同，只改扩展名） |
| 文档 | README、CHANGELOG、TypeDoc；版本对照表加 `1.1 ↔ 播放器 ≥ 0.2.0` |

### 7.2 推迟：`npm create vestra-plugin`

本轮不做。理由：它是另一个 npm 包（`create-vestra-plugin`），要单独的发布流程；模板内容（`name`、`author`、`icon`、README 骨架、id 前缀）等本提案定稿后才稳定；S4/S5 不依赖它。建议 S5 之后与插件仓库的贡献说明一起做。

### 7.3 兼容性审查

全部是新增：新方法、可选参数、输出接口的新属性、新库文件、新 wire 方法。逐项检查过的隐性破坏：

| 点 | 是否破坏 | 处理 |
| --- | --- | --- |
| **参考 §3.4「`<数据根>/<id>` 推出别的插件的数据目录」** | **是**（文档承诺的规则失效） | 不得不：GUID 目录的目的就是不再按 id 定位。替代：同一版本新增 `PluginInfo.dataDirectory`。唯一使用者是插件管理（`manager.ts` 的 `dataDirectory(id)`），随本轮迁移；参考与 SDK 注释（`PluginInfo.directory` 上的「`<data root>/<id>`」）同步改。未发版 |
| 数据与密钥位置变化 | 对插件代码否；对用户数据是（一次性丢失开发期数据与登录） | 1.5a 已定不迁移；通知 youtube、bilibili（登录密钥） |
| `install()` 不带参数遇到其它来源的同 id → `conflict` | 错误语义新增 | 今天不存在「其它来源」（没有商城、没有链接安装），所以对现有调用方没有行为变化；工坊本轮改为传 `source: workshop` 并处理 `conflict` |
| `uninstall` 新增 `unsupported` 情形、`install` 新增 `unsupported`（内置） | 否 | 两个方法已有 `unsupported`，调用方本来就要处理 |
| `PluginInfo.origin` 不加 `"builtin"` | — | 刻意避免（§2.2） |
| TestHost 条目类型加字段 | 否 | 新字段可选 |
| `pack` 输出 `.zip` → `.vxp` | CLI 行为变化，非插件 API | S0 已注明暂定格式；SDK CHANGELOG 写明；通知 S3 |
| `apiVersion` | 不变 `vestra/1` | |

---

## 8. 影响与通知清单

| 对象 | 需要知道 | 需要做 |
| --- | --- | --- |
| 插件管理 / 商城（插件仓库中的 `plugins/plugins/`） | `dataDirectory`、`builtin`、`source`；`validateArchive`；`install({ source, replace })` 与 `conflict`；Markdown 组件 | 数据目录改读 `info.dataDirectory`；内置禁用卸载；显示来源；S5 起用新接口 |
| 插件工坊（`plugins/workshop/`） | `install` 可能 `conflict`（与手动安装或仓库安装撞 id）、`unsupported`（与内置撞 id） | 传 `source: { kind: "workshop" }`；`conflict` 时确认后 `replace: true`；`unsupported` 提示改 id |
| youtube、bilibili | `secrets` 改按实例存，现有登录要重新登录一次 | 无代码改动 |
| 全部插件作者 | 数据目录位置变（开发加载在 `plugin-data/development/<id>/`）；`build` 会把 `README.md` 打进包，README 将显示在商城详情页；新的可选显示字段；用到 1.1 新 API 时写 `minPlayerVersion` | 进官方仓库前补 `name`、`author`、`punchline`、`icon`，把 README 改成面向用户 |
| S3（插件仓库 CI） | `pack` 产出 `.vxp`；`index.json` 版本条目建议加 `minPlayerVersion`；播放器侧维护内置插件清单 | 按 `.vxp` 上传 Releases |
| 协调者 | §9.3 真机验证点；需要拍板的问题（报告） | |
| 文档（维护者自己） | 参考 §2、§3.4、§6.1（新增 Markdown 小节）、§12、§13（`versionMismatch`）、§17 变更说明；架构总览代码地图 | 随实现更新 |

---

## 9. 实现分工、顺序与测试

### 9.1 顺序（维护者一人；括号内为可并行处）

| 步 | 内容 | 依赖 |
| --- | --- | --- |
| A | 宿主：`manifest.rs` 合并两个 manifest 结构，新字段校验与 `minPlayerVersion` | 无 |
| B | 宿主：catalog `instance` / `dormant`、GUID 数据目录、开发加载目录、`secrets` 按实例、内置保护、`source`/`replace`、`packages.list` 新字段 | A |
| C | 宿主：`validateArchive`（解压，见下） | A（与 B 并行） |
| D | 宿主：`builtin.json` 播种 + `src/scripts/stage-plugins.mjs`（从插件仓库 Releases 或本地目录取 `.vxp`，按清单校验 sha256） | B、C；S3 有 Releases 前用本地 `.vxp` |
| M0→M | Markdown：探针 M0（§5.4）→ 解析与模型 → `markdown.slint` → `prop_value` 分支 → `ui.markdownScan` | 与 A–D 并行 |
| S | SDK 1.1.0：先发类型与 TestHost（可用 `1.1.0-rc.1` 给 S5 作者提前写代码），再补实现与 CLI | 线上协议随 B、C、M 定 |
| P | 播放器升级 SDK 依赖、host-agreement 测试加规则表比对、文档；通知插件管理与工坊作者迁移 | S |

`.vxp` 解压：建议用 `zip` crate（MIT，`default-features = false`，只开 deflate，锁精确版本），不手写 zip 解析——这是处理外来输入的安全敏感代码，用成熟库更稳。限制与 `package_stage` 一致并在**解压过程中**强制：归档 ≤ 64 MB；条目 ≤ 1024；单文件解压后 ≤ 32 MB、总计 ≤ 64 MB（防 zip 炸弹）；只接受 stored/deflate、不接受加密；拒绝绝对路径、`..`、`\`、符号链接（外部属性）、大小写不敏感重名；`manifest.json` 必须在根。新依赖属于「引入许可证义务」，见报告。

### 9.2 测试

| 层 | 内容 |
| --- | --- |
| 宿主单测 `package_store` | 新装生成 GUID；同来源升级沿用；异来源无 `replace` 拒绝、有则替换并转休眠；换回复用 GUID；内置不可卸载、不可替换；替换失败恢复 `instance`；`dormant` 上限；`enabled` 跨重开；旧 catalog（有 `disabled`、无 `instance`）读入 |
| 宿主单测 `plugins` / `system` | 数据目录按 `DataKey` 落在 GUID / `development/<id>` / 试载临时目录；`removeData` 删 GUID 目录；`secrets` service 随实例 |
| 宿主单测 manifest | 每个字段的边界（长度、控制字符、路径越界、icon 格式与尺寸、默认 README 不存在不报错、显式不存在报错、URL 协议）；未知字段拒绝；`minPlayerVersion` 先于严格解析报错 |
| 宿主单测 `.vxp` | 正常包与 `pack` 产物同 digest；`..`、绝对路径、符号链接、重名（大小写）、加密、超限、炸弹（高压缩比）全部拒绝 |
| 宿主单测 Markdown | 每种块的模型输出；HTML 丢弃；相对链接按 `base` 解析；超限截断；图片路径越界走 `ui_image` 的拒绝 |
| 宿主单测播种 | 首次播种；版本更高才升级；回退后不被同版本覆盖；其它来源已占 id 时跳过；sha256 不符跳过 |
| SDK 单测 | `validateArchive`、`install(options)` 的 wire 参数；`info()` 新字段；markdown 下载：协议、大小、类型、缓存淘汰、失败不抛（用本地 HTTP 服务器，不访问外网） |
| TestHost | 冲突与替换模拟；旧形态的 `host.packages` 条目仍可用；`host.markdown()` |
| CLI | `check` 新字段报错与无前缀提示；`build` 带上 README/icon；`pack` 产出 `.vxp` 且确定性 |
| 一致性 | `host-agreement.test.ts` 比对 manifest 规则常量；`sync-sdk-slint` 覆盖 `markdown.slint` |
| §6.1 全套 | 宿主 fmt/clippy/test；`src/` check、boundaries、test:sdk、plugins:build、各插件测试；SDK 仓库 CI；portability（Windows 目标编译） |

### 9.3 需要协调者真机验证

1. Markdown：在一个窗口里显示含中文、标题、列表、代码块、引用、表格、本地与远程图片、链接的文档；点击链接打开浏览器；滚动流畅；窗口缩放时重新换行。StyledText 路线（或降级）在 GUI 上的实际效果。
2. 拖入 / 选择 `.vxp` 安装（经插件管理或临时测试插件），安装后热启用。
3. 替换流程：同 id 从两个来源安装，确认框、替换、换回后数据还在。
4. 内置保护：打包布局下（`Resources/plugins`）首次启动播种全部内置插件；插件管理里内置插件「卸载」不可用、可停用、重启后保持停用。
5. 数据隔离：替换后新插件看不到旧插件的数据目录内容与密钥（macOS keychain、Windows 凭据管理器各一次）。
6. Windows：`.vxp` 路径与大小写、GUID 目录、密钥 service 名。

---

## 10. 刻意不做的

| 诱惑项 | 不做的理由 |
| --- | --- |
| 宿主下载 `index.json`、校验签名、检查更新、处理撤回 | 1.4 已定在商城插件；签名「以后再考虑移进核心」 |
| 宿主内置 HTTP 客户端（为远程图片） | 引入 TLS 栈与平台证书差异，只为一个显示功能；Node `fetch` 已足够（§5.3） |
| `PluginInfo` 带显示字段（name、icon…） | 插件可以直接读 `info.directory/manifest.json`（宿主已校验）；少一份要维护的投影 |
| 暴露 GUID、`installedAt` | 没有调用方需要（§2.5）；需要时再按 §4 请求加 |
| 休眠实例的列表与清理接口 | 先观察是否真有大量残留；需要时作为插件管理的请求单独设计。现在不自动删除用户数据 |
| 放宽 `ui.image` 让商城显示别的插件包里的图标 | 商城可把文件复制进自己的数据目录再显示；不为一个插件开放跨插件读路径 |
| `PluginInfo.origin` 增加 `"builtin"` | 输出联合加成员是隐性破坏（§2.2） |
| 宿主强制 id 前缀或保留 `vestra.` | 1.5a 已放弃 |
| 未知 manifest 字段改为忽略 | 失去拼写错误检查；`minPlayerVersion` 已解决向前兼容（§4.2） |
| `maxPlayerVersion`、版本范围 | 没有需求；`apiVersion` 管破坏性变更 |
| Markdown：内嵌 HTML、语法高亮、SVG/GIF/WebP、脚注、数学、编辑、目录跳转 | 1.6 第 10 条只要常见子集；每项都要额外解析或解码代码 |
| Markdown 专用能力名 | 只是属性值，显示位置已受能力约束（§5.6） |
| `.vxp` 文件关联（双击打开）、拖到主窗口直接安装 | 需要平台注册与安装确认界面；拖入安装由商城用现有 `ui.drop` 做 |
| `npm create vestra-plugin`、`vestra-repo build` | 前者推迟到 S5 后（§7.2）；后者属于 S3 |
| 插件沙箱、按能力限制联网与文件 | 1.6 第 4 条：以后再说 |
| 自动更新（不经用户确认） | 1.6 第 5 条：手动确认 |

## 协调者确认（2026-10-21）

- 提案整体通过，按 §9 的顺序在阶段 3 实现。
- 链接安装与本地文件安装算同一来源（侧载），按提案。
- Markdown 行内样式先做探针 M0；`StyledText` 在解释器里不可用时降级为纯文本，由维护者决定，不再请示。
- 密钥按实例隔离，YouTube、Bilibili 已有登录失效一次：接受（未发版）。
- 新增 `zip` crate（MIT）：用户已同意。发行包需附带其许可证文本，与其它第三方许可证一起整理。

## 协调者修订（2026-10-21，内置插件留在主仓库）

用户决定内置插件留在播放器仓库（生态计划 1.6 第 6 条修订），只有 `youtube`、`bilibili` 迁到插件仓库、经商城安装。对本提案的影响：

- 内置插件的来源是**安装包**（`source.kind = "bundled"`），不是官方仓库地址；`builtin.json` 与 `.vxp` 由打包脚本从 `src/plugins/` 构建。
- 第一版商城**不更新、不替换**内置插件：对内置 id 的任何 `install`（无论来源、带不带 `replace`）都返回 `unsupported`；只有播种（新安装包里版本更高）会升级内置插件。§3 表中「`install` 同来源（官方仓库）的新版本 → 升级」一行作废；§3.3 中「用户经商城装了更高版本则不动」一种情形不再出现。
- 回退、停用照常。

## 实现记录（2026-10-21，阶段 3，API 维护者）

按 §9 的顺序实现了 §0 的 1–8 项及协调者修订（内置插件来源 `bundled`、对内置 id 的任何 `install` 都是 `unsupported`）。播放器在 `rebuild/store-core` 工作树，SDK 在 `vestra-player-sdk` 的 `store-1.1` 分支（提交 `453088e`，版本 1.1.0，未打标签、未推送）。宿主版本升为 **0.2.0**（`Cargo.toml`），即 SDK 1.1 新增接口要求的 `minPlayerVersion`。

### 宿主

| 文件 | 内容 |
| --- | --- |
| `manifest.rs`（新） | 开发加载与托管包共用的 `Manifest`（`plugins::Manifest`、`package_stage::PackageManifest` 都是它的别名）与规则表（§4.1）；`parse` 先读 `minPlayerVersion` 再严格解析；`NeedsPlayer` 错误在 RPC 里映射为 `versionMismatch` |
| `package_store.rs` | `Installed.instance`（GUID、`source`、`builtin`、`installedAt`、`seededVersion`）、`Catalog.dormant`（≤ 256）、`Recovery.instance`；提交时按 GUID 判断升级/替换（替换：旧实例转休眠、`previous` 清空；换回：从休眠取回）；`uninstall(keep_data)`；旧 catalog 打开时补实例 |
| `package_manager.rs` | `install` 读 `source`/`replace` 决定实例（`install_instance`，§1.3/§2.3）；`packages.validateArchive`；`uninstall` 拒绝内置；`packages.list` 新字段；开发加载同 id 时优先；`seed_builtins` 播种 |
| `package_archive.rs`（新） | `.vxp` 解压（`zip` 9.0.5，只开 deflate），上限在解压过程中强制 |
| `plugins.rs` | `DataKey`（`Instance(guid)` / `Development(id)` / `Trial`）决定数据目录与密钥服务名 |
| `system.rs` | `secrets.*` 的服务名取自 `DataKey` |
| `ui_markdown.rs`（新）、`ui/vestra/markdown.slint`（新） | pulldown-cmark 0.13.4 解析成 `[MarkdownBlock]` 模型；`ui_image.rs` 的 `prop_value` 识别 `$vestraMarkdown`；`ui.markdownScan`（无能力）返回需要下载的远程图片 |
| `ui_values.rs` | `reconcile` 遇到非 JSON 值（Markdown 行里的图片与样式文字）时整体替换，不再报错 |

**M0 探针结论：采用 `StyledText`**。`slint::private_unstable_api::re_exports::{parse_markdown, string_to_styled_text}` 构造 `Value::StyledText`（解释器里该变体 `doc(hidden)`），插件 `.slint` 里的 `StyledText` 元素与结构体字段 `styled-text` 都能编译、渲染、`link-clicked` 可用（`ui_markdown` 的无界面渲染测试：文字、图片像素、点击链接回调）。为了不让用户文本进入 Slint 的 Markdown 解析，格式串里只有我们生成的标记与占位符 U+E541，文字全部作为占位参数传入；`parse_markdown` 包在 `catch_unwind` 里，失败时退回纯文本。Slint 锁定 `=1.16.1`，升级 Slint 时须复测这一处（`ui_markdown.rs` 的 `Inline::styled` 与渲染测试）。

**与提案不同的实现选择**

- 启用状态仍存在原 `disabled` 集合（按 id，只作用于当前实例），没有搬进 `instance.enabled`：语义相同、改动更小。
- 内置插件播种在启动时**直接提交**，不等激活成功；随后由启动流程像其它已安装插件一样启动，启动失败显示为 `failed`。若等激活，无界面或某次启动失败时内置插件会从列表里消失。
- 试载的数据目录与密钥服务名不变（本来就按 `<id>.trial.<owner>` 隔离）。
- `validateArchive` 的解压错误为 `invalid`（目录的 `validate` 出错仍是原来的 `host`）。
- 卸载时删除数据，但系统密钥库里该实例的条目不删（无法枚举服务下的条目）；新实例的服务名不同，读不到它们。

### 打包

`src/scripts/stage-plugins.mjs <目标>/plugins`：对 `src/plugins/` 下每个有 `manifest.json` 的目录运行 `vestra-plugin pack`，生成 `<id>-<version>.vxp` 与 `builtin.json`（`{ schema: 1, plugins: [{ id, version, file, sha256 }] }`；SDK 1.0 产出的 `.zip` 自动改名）。宿主从 `$VESTRA_BUILTIN_PLUGINS`，否则 `<exe>/../../Resources/plugins` 读取；没有 `builtin.json` 时不播种（开发运行）。youtube、bilibili 由另一个工作树删除后自然不再打包。

**新增依赖与许可证**：`zip` 9.0.5（MIT；新带进依赖树的只有 `typed-path` 与 Unix 上的 `nix` 0.31，均为 MIT 或 MIT/Apache-2.0，其余 `flate2`、`crc32fast`、`indexmap`、`memchr` 原已在树里）、`uuid` 1.28.0 与 `pulldown-cmark` 0.13.4（均已在依赖树里）。仓库目前没有汇总第三方许可证文本的位置，发行包附带许可证需在打包任务里统一处理。

### SDK 1.1.0

类型、`packages.validateArchive`、`install(options)`、`PluginInfo` 新字段、`ui.markdown`（`src/plugin/markdown.ts`：Node `fetch` 只下 `https:`、手动跟随 ≤ 5 次重定向、单张 4 MB、10 秒、并发 4、缓存 32 MB/24 小时）、`src/manifest.ts`（CLI 的规则表；`MANIFEST_LIMITS`、`MANIFEST_FIELDS`、`MARKDOWN_LIMITS` 经 `./runtime` 给播放器一致性测试）、TestHost、CLI、`slint/vestra/markdown.slint`、README / Getting started / `CHANGELOG.md`。全部是新增；兼容性审查同 §7.3。`npm create vestra-plugin` 按 §7.2 推迟。

### 文档与示例

`doc/plugins/reference.md`（§2、§3.4、§6.1、§12、§13、§15、§17）、`doc/plugins/store-changes.md`（给插件维护者的通知与例子）、`doc/plugins/README.md`、`doc/architecture/README.md`；示例插件 `src/examples/markdown/`（Markdown 窗口、显示字段、`minPlayerVersion`）。

### 检查（本次实际执行）

- 宿主（`CARGO_TARGET_DIR=~/.cache/vestra-target/core`）：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 通过；`cargo test --locked` 287/287（基线 263，新增 24）。`npm run portability`：clean，宿主可编译到 Windows 目标。
- SDK 仓库：`npm ci`、`check`、`format:check`、`build`、`npm test` 111/111（基线 105）、`npm pack --dry-run`（83 个文件）、`npm run docs`（警告即错误）通过。
- 播放器（临时 `npm install <SDK 仓库>`）：`check`、`format:check`、`boundaries` 通过；`test:sdk` 11/11；`plugins:build` 通过；19 个插件测试 552/552，与基线逐个相同；示例 bookmark 1、core-ui 2、panel-menu 1、markdown 1。交付前 `src/package.json`、`package-lock.json` 已恢复为 `^1.0.0`（与提交前逐字节相同）。
- 无界面端到端（`vestra-host --headless`，临时的安装库与数据根）：首次播种 19 个内置插件（来源 `bundled`、各自 GUID 目录）；再次启动不重复播种；新安装包里 2 个插件升版本后升级、GUID 不变、`previousVersion` 为旧版本、用户停用状态保留；一个 `plugins.manage` 驱动插件依次验证了 `validateArchive`、仓库来源安装、异来源 `conflict`、`replace` 替换（新 GUID）、换回原来源（复用原 GUID 与数据）、内置插件 `uninstall`/`install` 均为 `unsupported`、可停用、含 `../` 条目的包被拒、`minPlayerVersion: 9.0.0` 的包为 `versionMismatch`、保留数据卸载后转休眠；开发加载与已安装插件同 id 时只运行开发加载的。

### 发现但未修

- 安装库的操作回执上限 512 条（`MAX_OPERATIONS`）从不清理；每次播种升级也会写回执（同一包复用同一回执 id）。长期使用后安装会以「operation history full」失败，需要单独设计回执的淘汰。
- `npm run portability` 把 Windows 检查的产物写在仓库内 `src/host/target/portability`（已被 `.gitignore` 忽略），与「不在仓库里产生 `target/`」的约定不一致。
