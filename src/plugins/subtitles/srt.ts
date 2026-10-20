import { open } from "node:fs/promises";

const MAX = 8 * 1024 * 1024;

/** Read a local `.srt` file (1 byte – 8 MiB) and require strict UTF-8. */
export async function readSrt(path: string): Promise<Buffer> {
  if (!/\.srt$/i.test(path)) throw new Error("请选择 .srt 字幕文件。");
  const file = await open(path, "r").catch(() => {
    throw new Error("无法读取字幕文件。");
  });
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size < 1 || stat.size > MAX)
      throw new Error("字幕文件须为 1 字节至 8 MiB 的普通文件。");
    const buffer = Buffer.alloc(MAX + 1);
    let length = 0;
    while (length < buffer.length) {
      const result = await file.read(buffer, length, buffer.length - length, null);
      if (!result.bytesRead) break;
      length += result.bytesRead;
    }
    if (length < 1 || length > MAX) throw new Error("字幕文件为空或超过 8 MiB。");
    const bytes = buffer.subarray(0, length);
    try {
      new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      throw new Error("字幕文件不是 UTF-8 编码。");
    }
    return bytes;
  } finally {
    await file.close();
  }
}
