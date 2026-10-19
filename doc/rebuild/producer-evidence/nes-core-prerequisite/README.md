# JSNES / Thwaite：无设备核心前提

2026-10-18，实验材料，不是 NES 产品或完整 M4 验收。未运行宿主、producer transport、GUI 或音频设备；未修改共享 src、构建 Cargo 或安装 npm/全局依赖。

## 固定输入与真实执行

使用前一阶段从源码新构建的 [Thwaite ROM](../nes-fixture-preparation/README.md)，SHA256 `ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173`。官方 [jsnes@2.1.0](https://registry.npmjs.org/jsnes/2.1.0) 原始 tarball 已归档，SHA512 与 registry 精确 integrity 匹配后，仅将源码/声明/许可解到新的系统临时目录；不执行 npm install。原 Apache-2.0 许可见 jsnes-LICENSE。

首次 Python 获取因本机 CA issuer 配置失败，改用启用正常 TLS 校验的 curl 成功；未关闭证书验证或修改系统证书。精确包身份在 preparation.json，首次实际命令为 `curl --fail --location --max-time 30 --output doc/rebuild/producer-evidence/nes-core-prerequisite/jsnes-2.1.0.tgz https://registry.npmjs.org/jsnes/-/jsnes-2.1.0.tgz`，退出0。

## 两轮结果与实际画面

- 首轮 verify.mjs 实际退出0：360帧、287534个双声道样本回调，非零音频1630个。查看 title.png/game.png 后确认第一次 Start 只从版权页进入标题菜单；文件名 game 不代表已经进入游戏。本轮原始结果、状态与截图保留在本目录，不扩大结论。
- 第二轮 verify-game.mjs 实际退出0，原始材料在 active-game-run/。再按一次 Start 后，实际 game.png 显示城镇、树木与 Pine 的操作教学画面；不称已完成用户键盘操控或全部游戏流程。共540帧、431301个双声道48k样本回调，非零103711，其中第二次 Start 后非零102081，峰值0.762664794921875；画面124种实际摘要，非假帧或单幅示意图。
- 两轮均在释放八键后，通过真实 toJSON→JSON文本→新核心load同ROM→fromJSON→releaseAll，按同样90帧输入序列回放。每帧完整RGBA摘要序列、逐帧累计音频样本数与f32le音频摘要均精确相同。第二轮状态1325678bytes，低于设计16MiB上限；仅为核心状态回放，不是插件存档容器或原子文件发布验收。

原始 onFrame buffer 复制后逐通道转换，不保留核心可复用 buffer 的引用。转换采用锁定包 [官方 screen.js](https://github.com/bfirsh/jsnes/blob/cbad9f1db57dfb275b90ffa26f9c479c355d8d5c/src/browser/screen.js) 的实际 setBuffer 路径：原word低8位→R、再8位→G、再8位→B、alpha固定255。README表述ARGB与浏览器路径的BGR注释有差异，不能仅凭名称选择高位R或直接转换主机字节序。此处只声称匹配该版本官方浏览器输出，不声称校准硬件色彩。

## 复现与边界

精确依赖归档已在本目录。需要新建依赖目录时从仓库根执行 `python3 doc/rebuild/producer-evidence/nes-core-prerequisite/extract.py`，它会校验归档并更新 preparation.json 中的临时路径。再执行 `node doc/rebuild/producer-evidence/nes-core-prerequisite/verify-game.mjs`。两个实际 run 的结构化结果、stdout及FFmpeg离线RGBA→PNG命令/退出码均已保留；这些是原始实际结果，不要求为查看材料重跑。Node导入官方浏览器模块时的localStorage实验警告保留在原日志，不影响本例未调用浏览器功能的结果。

头部默认为固定JSNES实现的本轮时序，不是PAL/全ROM兼容证明。无声学、实时性能/10分钟稳定性、真实输入焦点/释放、producer sample clock/背压/控制ACK、Slint呈现或生命周期验收。程序运行耗时仅记录，不据此承诺60fps；真实插件仍待共同基线后统一实现。
