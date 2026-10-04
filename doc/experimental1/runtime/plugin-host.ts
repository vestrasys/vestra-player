// 仅用于 Slint 实验；本版本协议和运行方式与最终成品 API 会有很大差异。
// Rust 拥有原生 Slint UI；这个 Node 子进程仅运行 TS 插件业务逻辑。
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { createContext } from '../sdk/context.ts';
import type { PluginDisposer } from '../sdk/types.ts';

const pending = new Map<number, {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}>();
let nextId = 1;
let closing = false;
let dispose: PluginDisposer | undefined;
let queue = Promise.resolve();
const lines = createInterface({ input: process.stdin });
const send = (message: unknown) => process.stdout.write(JSON.stringify(message) + '\n');

// stdout 专用于协议；插件 console 日志不会破坏 JSON 数据流。
console.log = console.info = console.debug = (...args) => console.error(...args);

const dispatch = createContext(<T>(method: string, params: Record<string, unknown>) => {
  return new Promise<T>((resolvePromise, reject) => {
    const id = nextId++;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`宿主请求超时：${method}`));
    }, 30_000);
    pending.set(id, { resolve: value => resolvePromise(value as T), reject, timer });
    send({ type: 'request', id, method, params });
  });
});

function rejectPending(reason: string) {
  for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error(reason)); }
  pending.clear();
}

async function shutdown() {
  if (closing) return;
  closing = true;
  try { await dispose?.(); }
  catch (error) { console.error('实验插件清理失败：', error); }
  finally {
    dispatch.clear();
    rejectPending('插件已停止');
    lines.close();
    process.stdin.destroy();
  }
}

lines.on('line', line => {
  let message;
  try { message = JSON.parse(line); }
  catch { send({ type: 'fatal', error: '无效的宿主 JSON 消息' }); void shutdown(); return; }
  if (message.type === 'response') {
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    clearTimeout(entry.timer);
    if (typeof message.error === 'string') entry.reject(new Error(message.error));
    else entry.resolve(message.result);
    return;
  }
  queue = queue.then(async () => {
    if (closing) return;
    if (message.type === 'shutdown') { await shutdown(); return; }
    try {
      if (message.type === 'action') await dispatch.action(message.name, message.value);
      else if (message.type === 'playback') await dispatch.playback(message.state);
      else throw new Error(`未知宿主消息：${message.type}`);
    } catch (error) {
      console.error('实验插件操作失败：', error);
      await dispatch.context.ui.patch({ status: `操作失败：${String(error)}` }).catch(console.error);
    }
  });
});
lines.on('close', () => {
  rejectPending('宿主通信已关闭');
  if (!closing) {
    closing = true;
    dispatch.clear();
    // EOF 表示宿主已消失，不能再发送需要回复的清理请求。
  }
});
process.on('SIGTERM', () => { rejectPending('插件进程正在退出'); process.exit(0); });

try {
  const entry = process.argv[2];
  if (!entry) throw new Error('需要指定实验插件 index.ts 路径');
  const plugin = await import(pathToFileURL(resolve(entry)).href);
  if (typeof plugin.activate !== 'function') throw new Error('插件缺少 activate(ctx)');
  // activate 之前禁止 action 执行；启动阶段仍必须处理 RPC response。
  const startup = plugin.activate(dispatch.context);
  queue = startup.then((cleanup: PluginDisposer | undefined) => { dispose = cleanup; send({ type: 'ready' }); });
  await queue;
} catch (error) {
  send({ type: 'fatal', error: String(error) });
  rejectPending('插件启动失败');
  closing = true;
  dispatch.clear();
  lines.close();
  process.stdin.destroy();
  process.exitCode = 1;
}
