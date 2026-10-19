# 网络视频来源接入 C1

> 2026-10-18 集中审查 C2：入口归属、插件独立性及复用公共source方向通过，可进入工作基线后的实现。SITE-01是站点切片提出的局部缺口，不作为其它插件开工条件。自动过期恢复、登录凭据、任意多段/所有格式兼容不是新增全局门槛；先完成公开样本真实播放与用户要求的YouTube下载，分离流在实际样本需要时使用成熟组件，明确支持范围。本文proposed接口须由维护者与实际作者收敛，非已发布API。

日期：2026-10-18。状态：最小设计待集中审查；未安装工具、未执行网络视频播放/下载、未修改 SDK 或宿主。依据本轮工作流顶部纠偏、产品交互 B1、菜单 B1 及当前新 `src/sdk/source.ts`、`commands.ts`。与 [YouTube](plugins/youtube.md)、[Bilibili](plugins/bilibili.md) 一起审查，仅新增受影响路径，不重开其他插件方案。

## 产品路径与职责

用户可在“文件→打开网络内容…”粘贴网址并点“播放”；这是基础播放插件的通用入口。“文件→网络来源→YouTube…”和“文件→网络来源→Bilibili…”分别打开各站点的普通 Slint 工具窗口，可粘贴网址并播放。站点名仅用于这些来源选择入口与插件管理，不增加顶级菜单。YouTube 下载属于“视频→下载→YouTube视频…”；同一 YouTube 窗口切到下载区，不复制第二份任务状态。以上菜单为集中审查提案，尚未修改全局菜单表。

播放列表增加“文件→播放列表→添加链接…”及侧栏同 action 的按钮：接受普通 HTTP(S) URL，加入后仅选择，不自动播放。条目保存原始/规范网址、稳定行 ID 和公开标题；不保存短期 CDN 地址或凭据。选中任意 URL 行→公共 source 打开→注册的提供者解析→普通 mpv 媒体播放。可在列表内选择并播放 YouTube，无 YouTube 插件时保留该行，显示“此网络来源的提供者不可用”，可安装后重试。网址匹配、标题读取、质量选择不属于列表；列表只处理公共 loading/ready/error、条件打开与自然结束。Bilibili 若注册提供者也可经相同路径使用，无需列表分支。这里扩展 playlist C2 的“仅本地文件”范围；由列表作者窄修其设计和实现，本文件不改其代码。

| 所有者 | 负责 | 禁止 |
| --- | --- | --- |
| 基础播放/列表消费者 | 提交原网址、显示公共状态；列表自己决定顺序 | import 站点插件、调用站点私有命令、枚举站点对象或自取 CDN 地址 |
| 站点 TS 插件 | 注册声明式 URL 匹配、yt-dlp 子进程、站点解析、格式选择、短链校验、凭据、重试/重解和下载 | 读取列表对象/实现；把业务移入 Rust；无用户动作扫描浏览器 cookie |
| 公共 source | 注册/能力发现、匹配与冲突、调用关联、取消/超时、来源条件、提交媒体及生命周期 | YouTube/Bilibili ID、域名、站点 API、签名、下载策略分支 |
| mpv/成熟媒体组件 | 解复用、解码、音视频同步、网络媒体读取 | 让宿主自己拼站点 DASH 解码器 |

提供者声明自己能处理的网址模式；宿主通用匹配器只解释数据。不在 core 内写域名表。无匹配的直接媒体 HTTP(S) 可走通用后端；被提供者声明处理的网页不能失败后悄悄交 mpv 内置 ytdl 再解析。多个同优先级提供者匹配时返回可诊断冲突，不广播网址给全部插件。提供者停用撤销注册及未完成解析；已交 mpv 的普通媒体可继续播放，后续需要重解时显示提供者不可用。

## 最小公共缺口与 usage

当前 `source.submit({kind:"open",source:string,startPaused,...})`、`source.state()`、`source.operation(id)`、stamp/cycle/eventSequence 已有；`openProvider.producerId` 是音视频生产者，不是 URL resolver，不能冒用。当前 source 类型没有 resolver 注册、解析请求/回送及多流/请求头的媒体描述。`commands.register` 已能贡献路径与状态，dialogs 已有目录/保存面板，不请求另建网络工具 UI 框架。

**SITE-01：通用 URL 提供者与媒体描述的最小闭环。** 只请求维护者补齐当前真实路径：公开注册匹配和能力；宿主在既有 open 内触发有 request/operation/ticket 关联的 TS resolve；插件返回有类型的媒体描述（主 URI、可选外部音轨 URI、必要请求头/受限 cookie 引用、公开标题/原网址）；宿主经 mpv 现有设施装载。具体签名由维护者确定。主/音轨请求上下文必须可表达或明确拒绝不支持的组合；不提供任意 mpv 命令/选项注入字典。私有的解析结果只给宿主，不向列表广播带签名的 URI、header/cookie。公共 source 身份应指向原网址/描述身份，而不是把 CDN URI 当唯一持久身份。

```ts
// 当前 SDK 的真实消费者形态；路由能力尚缺，不能据此声称网页可播。
const opened = await host.source.submit({
  kind: "open", source: enteredUrl, startPaused: false,
}, operationId);
// ready/error 仍通过公共 source.state 及既有 ticket/stamp/cycle 关联。
// 等待超时后查 source.operation(operationId)，不盲目重复 open。

// 以下仅语义草图，不是已存在的 SDK 名称或冻结接口：
proposed.registerUrlResolver({ patterns: declaredPatterns }, async request => {
  const info = await pluginExtract(request.url, request.signal);
  return pluginSelectMedia(info); // typed media descriptor; no queue objects
});
```

接受 source 意图后先表现 loading，再等待 provider 与 mpv 的真实 ready。切到新来源/关闭时以同一公共条件使旧 resolve 失效，迟到结果不得装载；不能解析完再用无条件第二次 open 覆盖用户较新的来源。列表自动下一项继续传既有 expected stamp；该原子检查必须覆盖解析后的实际提交，不能只在解析开始检查。调用超时不等于来源已取消；公共 operation 查询恢复结果。站点工具的“取消解析”只有公共取消被确认后才显示已取消，已装载媒体通过公共关闭动作处理。维护者需确认最小 resolve-cancel/result 查询如何接入现有 operation，而非另造任务平台。

**SITE-02：当前来源发生网络读取失败后的通用重解通知/受条件约束重开。** 解析前后短时失效由站点插件在原 request 内至多重解一次；播放中失效由公共结构化失败和 stamp 关联交回其提供者，插件决定是否重解及提出受当前 stamp 约束的媒体替换。Rust 不解析签名查询参数或推断站点有效期。不要求无缝续播：成功时在来源仍未更换且可 seek 时恢复公开确认的位置、暂停状态；否则明确失败并提供“重新播放”。若当前契约暂不足，先交显式重试路径，将自动恢复标为待实现，不阻塞普通公开视频纵向切片。

## 失败、私密信息与资源

| 情况 | 插件和公共设施的行为 | 产品反馈 |
| --- | --- | --- |
| 不合法/不支持的网页 | 插件限制 HTTP(S)、校验输入及短链最终域名；宿主通用路由明确无提供者 | 保留输入，显示不支持或提供者缺失；不改当前来源直到接受新意图 |
| 登录、地区、付费、DRM 或访问拒绝 | 提供者返回可展示分类；不付费、不绕访问控制 | 说明当前视频不可用，不承诺全部地区/清晰度/视频 |
| 工具缺失/版本不兼容 | TS 做工具路径/版本/依赖检查，注册能力可报告不可用 | 普通文件仍可播；站点播放/下载明确具体缺依赖 |
| DNS、断网、解析超时 | 插件有限超时/重试并终止所属进程；宿主记解析终态/结果 | 区分已失败、正在取消、结果未知；用户可明确重试 |
| URL 失效、headers 不满足 | TS 从 extractor 输出构造必要上下文并重解；mpv 读取错误关联当前来源 | “网络内容已失效/无法读取，重试”；不展示签名/过期查询参数 |
| cookies | 默认匿名；本轮不读取用户 cookie/密钥。未来显式选择文件仅由所属站点管理，不进入列表、菜单状态、日志 | 登录依赖作为不支持说明；不是自动读取浏览器的理由 |
| 停用/崩溃 | 撤销 resolver，终结所属解析进程/订阅；宿主令 pending request 失败；清理私有临时数据 | 已交付 mpv 的媒体无需重启；依赖该 provider 的重解不可用 |

请求头按来源/流限定生命周期，换源必须清除；不得用全局 mpv header 残留将 Bilibili Referer 或凭据带到下个网址。网络凭据只向正确域名/重定向策略允许的目标发送。没有每流 header 能力时不得用“全局 Cookie”假装等价，应返回缺能力；首个匿名公开样本可先用共同非敏感请求头验证。

## 官方资料调查与工具分发

下列官方项目资料均于 **2026-10-18** 查阅；master/stable 页面会变化，实现时记录实际工具版本/构建校验值，不以本文宣称某版本已经运行。

- [yt-dlp README](https://github.com/yt-dlp/yt-dlp)：支持机器可解析的 `-J`、`--print`、`--progress-template`，应避免解析普通控制台文本；有 `--no-overwrites`；ffmpeg/ffprobe 用于合并及后处理；官方提供多平台产物。TS 通过参数数组执行子进程、`--ignore-config` 和 `--no-plugin-dirs` 隔离隐式配置/额外 extractor；URL 放在 `--` 后，不经 shell 拼接。
- [yt-dlp EJS 官方指南](https://github.com/yt-dlp/yt-dlp/wiki/EJS)：YouTube 需要外部 JavaScript runtime 与 EJS；官方可执行包包含 EJS，第三方打包需检查；Deno 默认启用且推荐。插件必须声明并检查这一依赖，不能假定宿主的 TS 运行时必能直接代用。
- [mpv stable 手册](https://mpv.io/manual/stable/#network)、[外部音轨](https://mpv.io/manual/stable/#audio)：支持 HTTP headers、Netscape cookies 文件、外部音频；这支持“主视频+音轨”描述方向，不证明当前 SDK 已暴露它。mpv 的网络/音轨能力由通用 backend adapter 封装，站点不直接取得后端控制句柄。
- [mpv EDL 官方格式](https://github.com/mpv-player/mpv/blob/master/DOCS/edl-mpv.rst)：可表示多段/多流时间线；若 Bilibili 返回分段，由插件用成熟组件生成合法表示，严格处理长度转义，宿主仍只接收通用媒体描述。首片段不足以代表完整视频。
- [yt-dlp 官方支持列表](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)及 [Bilibili extractor](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py)：有 Bilibili 来源处理，extractor 返回格式及 Referer 语义；支持列表不保证具体 URL 当前可用。
- [yt-dlp Licensing](https://github.com/yt-dlp/yt-dlp#licensing)：项目源码许可证与打包依赖不同；PyInstaller 组合产物含 GPLv3+ 代码。离线分发必须按实际包附许可/归属和相应材料，不能仅标 Unlicense。

工具可预先打包供离线安装/启动依赖检查；实际站点解析/播放/下载仍需网络。macOS/Windows 可选官方独立包；Linux 可选对应架构独立包或 Python 包，通用 zipimport 产物需 Python。按 OS/架构及系统库要求分发，ffmpeg 构建许可另核对。JS runtime、EJS、ffmpeg 作为明确的插件依赖管理；缺失返回具体错误，不在播放中自动安装或自更新。插件可共享受版本管理的外部工具安装位置，但不互相 import 或依赖对方进程/缓存。

## 第一轮可观察验收

先审最小设计，再按共同基线准入进入产品切片，不等所有网络地域实证。只选公开、无需登录且允许使用的样本；本文没有现场成功记录。

1. 两插件分别独立激活：打开窗口、粘贴 URL、播放，主画布真实音画；缺另一插件/列表不影响它。
2. 通用“打开网络内容”及列表“添加链接→选择→播放 YouTube”都到同一公开 resolver；列表只见原网址和公共状态；读依赖和事件确认无兄弟导入/专用通信。
3. 分离音视频样本有声有画、可暂停/seek；Bilibili 必须覆盖完整内容，不能仅有画面或第一段。
4. 解析期间手动开本地媒体、关闭或停用；迟到 resolver 不夺回来源；来源切换清除 header，列表只在自然结束推进。
5. 缺工具、无 provider、坏网址、断网、超时、受限内容与过期重试分别有真实失败/取消反馈；原生不可用项明记待验。
6. YouTube 下载覆盖进度、取消、目标重名无覆盖、关闭窗任务继续及停用清理；下载不自动切换正在播放的来源。

当前最小实现阻塞是 SITE-01；SITE-02 自动恢复可分阶段交付。安装回退及 NES 清理是协调者既定顺序，不由本设计追加全局门槛。
