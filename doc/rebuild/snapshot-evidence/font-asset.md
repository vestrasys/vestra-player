# M3 固定中文字幕字体候选资产

本轮仅取得一个未经修改的 Regular 字体及对应许可证，供后续有限字体表、字体租约和覆盖检查接入。没有改变默认字体、样式映射、backend、SDK 或构建配置；没有运行 renderer、fontselect、构建或 GUI。

- 官方仓库：[notofonts/noto-cjk](https://github.com/notofonts/noto-cjk)。
- 固定 commit：`f8d157532fbfaeda587e826d4cd5b21a49186f7c`（不在运行时跟随 main）。
- 上游文件：[NotoSansCJKsc-Regular.otf](https://raw.githubusercontent.com/notofonts/noto-cjk/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf)。
- 本地文件：`src/assets/fonts/noto-cjk/NotoSansCJKsc-Regular.otf`。
- 格式：单 face OpenType/CFF，face index `0`，非 TTC、非 variable；未取其他语言文件、字重或整仓。
- sfnt name 表：family `Noto Sans CJK SC`；style `Regular`；PostScript `NotoSansCJKsc-Regular`。
- 字体版本：`Version 2.004;hotconv 1.0.118;makeotfexe 2.5.65603`。
- 大小：`16437364` bytes，约 15.68 MiB。此为随包资产，不应塞进字幕内容上传的 2 MiB 通道。
- SHA-256：`2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`。

对应固定 commit 的 [Sans/LICENSE](https://raw.githubusercontent.com/notofonts/noto-cjk/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/LICENSE) 已原样保存在字体旁。许可证为 **SIL Open Font License 1.1 / OFL-1.1**，允许按其条件随软件捆绑；分发须保留版权声明和许可证，字体不能单独售卖，修改版命名另受其条件约束。字体内版权声明为 `© 2014-2021 Adobe (http://www.adobe.com/).`，原始字体未修改，声明也记录在 `metadata.json`。

LICENSE 大小 `4301` bytes，SHA-256：`6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2`。同目录 `metadata.json` 保存固定来源 URL、提交、字节大小、两个摘要和实际 name 表字段。

校验使用 Python 标准库 SHA-256，并直接读取下载字体的公开 sfnt name 表；没有用 family 名推断实际后端选中结果。下载经 curl 正常 HTTPS 验证完成；Python urllib 的本机 CA 配置失败后没有关闭证书验证。

待维护者验证：将固定字节纳入真实 font lease、对每次字幕内容做逐 code point 覆盖检查、确认 mpv/libass 实际 face/index 与无意外 fallback，并核离线 FFmpeg 相同字体输入和呈现。不同后端的 provider 设置不同；`fontsdir` 或同 family 并不单独证明精确字体锁定。本资产不证明所有中文/Unicode 都覆盖，也不证明字体已被选择、已实现 font lease 或实时/离线逐像素一致。
