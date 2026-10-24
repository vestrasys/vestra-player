# Vestra Player AI（实验性）

播放器里的 AI 助手，基于 [pi](https://github.com/earendil-works/pi)。用你自己的 API key，和你选的模型对话；AI 可以在当前对话的文件夹里写 Vestra 插件，自己做类型检查、跑测试、打包，写好的插件可以试载、安装或导出。

## 怎么用

1. 右键菜单选「Vestra Player AI…」。
2. 第一次使用时点「去设置」（或侧栏的「⚙ 设置」），在「选项 ▸ 插件 ▸ Vestra Player AI」里：
   - 选服务商：pi 支持、用 API key 的全部服务商（Anthropic、OpenAI、Google、OpenRouter、DeepSeek、Mistral、Groq、xAI、MiniMax、Moonshot 等，按量付费），或者「自定义 · OpenAI 格式」/「自定义 · Anthropic 格式」（填 Base URL，可接 Ollama、LM Studio 或其它兼容服务）；
   - 粘贴 API key，点「保存」；每个服务商的 key 分开保存；
   - 选模型。自定义服务在填好 Base URL 后按 Enter 或点「获取模型」，会向服务列出模型；列不出来时直接填写模型 ID。
3. 回到 AI 窗口输入消息，Enter 发送，Shift+Enter 换行。生成过程中可以点「■ 停止」。
4. AI 工作时有一行状态：先显示「思考中…」，调用工具时显示当前的工具（最新的一次替换上一次）；这一轮结束后点这一行可以展开所有工具调用。
5. 左侧是对话列表：「+ 新对话」开始新对话，点一项切换。关闭后再打开，回到上次的对话。
6. 「打开文件夹」打开当前对话的工作文件夹。
7. 在设置页的「插件作者」里填你的 GitHub 用户名：AI 新建的插件 id 是 `<用户名>.<名字>`（官方插件仓库要求 id 前缀就是源码仓库所属的 GitHub 用户或组织）。不填时用 `local.<名字>`，只适合在本机使用，发布前要改 id。`vestra.` 留给官方插件，不能用。

## 写插件

让 AI 写插件时，它先读插件 SDK 的参考资料（只读，在插件包里），然后用这些工具，状态行里显示中文名称：

| 工具 | 状态行 | 做什么 |
| --- | --- | --- |
| `scaffold_plugin` | 创建插件 | 按官方插件仓库的布局新建项目：manifest.json、main.ts、ui/、tests/、README、CHANGELOG |
| `typecheck` | 类型检查 | 严格类型检查（播放器自带的 SDK 类型），错误带文件、行、错误码 |
| `test` | 运行测试 | 在单独的受限 Node 进程里跑 `tests/*.test.ts`：只能读插件文件夹，不能联网、不能开子进程 |
| `check` | 检查 | id、依赖、manifest、类型、构建（Slint 由播放器在试载或安装时编译） |
| `pack` | 打包 | 生成 `dist/<id>-<版本>.vxp` |
| `trial_load` | 试载 | 播放器校验后临时加载（只在你要求时） |
| `install_plugin` | 安装 | 先弹对话框确认，再安装（只在你要求时） |

类型检查、测试、检查都通过，AI 才算写完。每个工具都有时间上限、输出上限，点「■ 停止」会取消正在运行的工具。

对话里有插件项目时，输入框上方出现插件卡片：

- 「试载」：播放器校验后临时加载，用完点「停止试载」。播放器的试载只支持 `commands`、`ui.window`、`events` 三种能力，用到其它能力的插件请直接安装。
- 「安装」：安装进播放器（来源记为 AI 工坊）；同 id 的插件来自别处时先问是否替换。之后可以「回退」（有旧版本时回到旧版本，第一次安装则卸载）。
- 「导出 .vxp」：打包后另存，可以分享。
- 「导出源码 zip」：完整的插件项目，可以直接建成你自己的 GitHub 仓库：README、CHANGELOG、测试、tsconfig、带 SDK 依赖的 package.json、GitHub Actions 工作流（check、test、pack），以及待替换的 LICENSE 占位文件；不含 node_modules 和 dist。
- 「打开文件夹」：插件项目的文件夹。

## 说明

- **key 的保存**：只保存在系统钥匙串（macOS）或凭据管理器（Windows）里，不写入任何文件、日志或对话记录。调用模型的费用由你的服务商账户承担。
- **读取 key**：打开 AI 窗口时才读取，读过一次就记在内存里。系统可能会弹出授权对话框（播放器更新后常见）；拒绝或读取失败时，窗口顶部会提示「无法读取钥匙串中的 key，请在设置中重新保存」，其它功能不受影响。
- **AI 能做什么**：只能读写当前对话的工作文件夹（`read`、`write`、`edit`、`ls`、`find`、`grep`），另外可以读插件 SDK 的参考资料（只读）；不能运行命令，不能访问其它文件，也不能使用符号链接或以「.」开头的文件名。AI 在一个单独的受限进程里运行。插件工具由插件进程按固定参数执行。
- **对话保存在哪里**：插件数据目录下的 `sessions/<对话>/`：`meta.json`、pi 的对话记录（`.jsonl`）、工作文件夹 `work/`（每个插件一个 `work/<id>/`）和打包结果 `dist/`。数据目录的 `node_modules/` 是插件 SDK 的一份副本，供类型检查和测试使用。
- 同一时间只有一个对话在生成。
- AI 的回复按 Markdown 显示（标题、列表、表格、代码块等），代码块右上角可以「复制」，链接用浏览器打开。回复里的网络图片不会下载，只显示替代文字。很长的对话只显示最近的消息，完整记录在会话文件夹里。
- 服务商列表只有按量付费的 API；编程套餐、Token Plan 等订阅套餐不在列表里，需要时用「自定义 · OpenAI 格式」或「自定义 · Anthropic 格式」接入。
- 需要 Vestra Player 0.4.1 或更新版本。
- 插件 SDK（`vendor/@vestra-player/sdk`，含 `vestra-plugin` 命令行）、TypeScript 和 `@types/node` 随插件一起提供，与播放器使用的 SDK 版本相同（`npm run vendor:sdk` 更新，`tests/vendor-sdk.mjs`）。SDK 自带 `llms.txt` 之前，参考资料 `knowledge/` 由同一脚本从 SDK 的 README、.d.ts 和示例生成。
- 评估：`tests/eval/` 有 6 道固定题目，`npm run eval` 用真实模型跑（会花你服务商账户的钱，只在改提示词或工具后手动跑），见 `tests/eval/run.mjs` 开头的说明。
- 第三方组件的许可证见 `licenses/`，pi 的 logo 出处见 `licenses/pi-logo/NOTICE.md`。
