# 快照与导出前置的有限 CLI 实验

2026-10-17，root仅使用本轮自建素材。**尚未实现或验收 Rust snapshot/job API。** 此处的命令、原始产物和可复现脚本只支持下面的有限结论，不代替M3全部要求。

## 文件对象租约候选

运行 `python3 doc/rebuild/snapshot-evidence/source-lease-probe.py`，退出0；[result.json](source-lease-cli/result.json)保存命令和全部检查。

接受时打开红色视频的只读普通文件对象；随后把原路径替换成蓝色视频，并删除旧对象的最后一个路径。以持有的 File 作为子进程stdin，FFmpeg固定读取 `/dev/fd/0`，能够在0.5秒处seek并导出旧红色 `[254,0,0]`；按原路径重读则得到蓝色 `[0,0,255]`。设备/inode/大小/mtime记录证明持有对象未变，旧对象link count为0。实际 [leased.png](source-lease-cli/leased.png) 与 [path.png](source-lease-cli/path.png) 已保存。

同一次实验也确认dup句柄共享读取位置：在副本seek到7，原句柄位置同为7。不能据此把Rust `File::try_clone`当作并发独立reader；任务需各自接受时持有独立打开对象，或对同一个snapshot序列化读取。没有为此扩展通用文件系统事务。

此结果只证明Python/POSIX子进程和本机FFmpeg读取能力。原地写入内容的检测、Rust资源生命周期、任务取消/重试和发布前验证仍待实现。文件描述符保活不等于内容不可变。

后续实际宿主正在验证双独立打开的路线：一份仅供实时 libmpv，另一份供离线串行 Reader。锁定 [mpv 0.41 stream_file.c](https://raw.githubusercontent.com/mpv-player/mpv/v0.41.0/stream/stream_file.c) 的 `fd://` 直接使用传入 fd、不取得关闭所有权，打开时探测长度并回到偏移0；宿主须保活该 File，且不得让 worker 与实时播放器共享同一 open description。源码说明是候选依据，实际 Rust 宿主读、seek、同源 restart、替换路径后仍读旧对象的结果另待独立验证，不以源码代替通过。

## 离线构建能力必须实查

[offline-capabilities.json](offline-capabilities.json)保存本机两个现成构建的实际版本/滤镜清单：PATH中的FFmpeg9.0.2没有 `ass`/`subtitles`，`/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg` 同为9.0.2且有上述能力及本轮typed音视频滤镜。没有安装新包、改链接或改全局环境。不能仅看版本号就假设导出具备字幕；最终分发必须验证自己的成熟后端产物。

## 真实派生字幕 PNG

运行 `python3 doc/rebuild/snapshot-evidence/offline-subtitle-probe.py`，使用明确的FFmpeg-full路径及只读文件stdin，应用前一字幕实验的不可变派生WebVTT，实际导出 [processed.png](offline-subtitle-cli/processed.png)，退出0。root通过 `view_image` 实见 `<b>`、ASS花括号及字面 `\N`，没有被当成格式或换行。[结果](offline-subtitle-cli/result.json)与完整FFmpeg日志同目录。

实际日志为libass0.17.5/CoreText，派生字幕PlayRes384×288，解析到系统Arial.ttf。没有冻结或分发系统字体。与mpv默认CLI截图不逐像素相同；所记比较还包含字体/样式/色彩差异，不能把全部差异归为字幕，也不能宣布实时/离线已对齐。mpv的720参考单位不能直接复制到这个ASS288参考平面。

尚未验证：冻结完整视频链/字幕样式/字体/音频链和用户偏移、确定性preroll、来源切换或owner撤销后的不变产物、真实MP4音轨、取消/清理及无覆盖发布。FFmpeg的 `-n` 本身不是临时输出完成后发布的完整事务。

## 完整预热与加工片段候选

后续运行 `python3 doc/rebuild/snapshot-evidence/processed-preroll-probe.py`，退出0；[完整结果](processed-preroll-cli/result.json)、[实际MP4](processed-preroll-cli/processed.mp4)、解码PCM、错误对照、ffprobe和命令均保存。此项补充了上一实验未测的有限管线能力，不追改其原边界。

自建源含幅度突变的48kHz立体声和视频时间码。实际导出从源0开始处理，使用与当前typed节点一致的dynaudnorm 100ms/5帧、视频eq/gblur及派生VTT，加入 +0.2 秒用户音频延迟，再截取媒体2.3..4.3秒。volume/EQ/treble为中性值，只验证排列和动态历史，未冒称这些节点的独立效果。

- 实际成品为H.264/AAC、两秒、60视频帧；解码音频192000个交错样本，恰为48kHz×立体声×两秒。
- 独立PCM参考从完整源0处理后，按延迟对应的源2.1..4.1秒截取。成品AAC与参考的归一化RMS误差为2.518%，这包含有损编码，不宣称逐样本相同。
- 错误对照先截取源2.1..4.1再启动规范化，丢掉历史；相对完整预热参考误差14.092%，首400ms为16.460%。本素材下历史差异确实可观测，不只是滤镜命令返回成功。
- root通过 `view_image` 查看[首帧](processed-preroll-cli/first-frame.png)，可见实际媒体2.300秒、69帧时间码、加工画面与字幕。

仍是CLI管线实证，不是已实现的snapshot/job。字幕字体/样式尚未冻结或与实时对齐，没有复制实时DSP内部状态；来源/owner变更后的不变产物、取消、清理和临时输出无覆盖发布仍待真实宿主实现。非零PCM没有被写成设备可听/声音连续的证明。
