# Vestra Player AI（实验性）

播放器里的 AI 助手，基于 [pi](https://github.com/earendil-works/pi)。用你自己的 API key，和你选的模型对话；AI 可以在当前对话的文件夹里读写文件。

## 怎么用

1. 右键菜单选「Vestra Player AI…」。
2. 第一次使用时点「去设置」（或侧栏的「⚙ 设置」），在「选项 ▸ 插件 ▸ Vestra Player AI」里：
   - 选服务商：pi 支持、用 API key 的全部服务商（Anthropic、OpenAI、Google、OpenRouter、DeepSeek、Mistral、Groq、xAI、MiniMax、Moonshot、Z.AI 等），或者「自定义 · OpenAI 格式」/「自定义 · Anthropic 格式」（填 Base URL，可接 Ollama、LM Studio 或其它兼容服务）；
   - 粘贴 API key，点「保存」；每个服务商的 key 分开保存；
   - 选模型。自定义服务在填好 Base URL 后按 Enter 或点「获取模型」，会向服务列出模型；列不出来时直接填写模型 ID。
3. 回到 AI 窗口输入消息，Enter 发送，Shift+Enter 换行。生成过程中可以点「■ 停止」。
4. AI 工作时有一行状态：先显示「思考中…」，调用工具时显示当前的工具（最新的一次替换上一次）；这一轮结束后点这一行可以展开所有工具调用。
5. 左侧是对话列表：「+ 新对话」开始新对话，点一项切换。关闭后再打开，回到上次的对话。
6. 「打开文件夹」打开当前对话的工作文件夹。

## 说明

- **key 的保存**：只保存在系统钥匙串（macOS）或凭据管理器（Windows）里，不写入任何文件、日志或对话记录。调用模型的费用由你的服务商账户承担。
- **读取 key**：打开 AI 窗口时才读取，读过一次就记在内存里。系统可能会弹出授权对话框（播放器更新后常见）；拒绝或读取失败时，窗口顶部会提示「无法读取钥匙串中的 key，请在设置中重新保存」，其它功能不受影响。
- **AI 能做什么**：只能读写当前对话的工作文件夹（`read`、`write`、`edit`、`ls`、`find`、`grep`），不能运行命令，不能访问文件夹以外的文件，也不能使用符号链接或以「.」开头的文件名。AI 在一个单独的受限进程里运行。
- **对话保存在哪里**：插件数据目录下的 `sessions/<对话>/`：`meta.json`、pi 的对话记录（`.jsonl`）和工作文件夹 `work/`。
- 同一时间只有一个对话在生成。
- 这一版是第一阶段：纯文本消息（还不渲染 Markdown），生成、校验、安装插件会在后续版本加入。
- 第三方组件的许可证见 `licenses/`，pi 的 logo 出处见 `licenses/pi-logo/NOTICE.md`。
