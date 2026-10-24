import type { Plugin } from "@vestra-player/sdk";
import path from "node:path";
import { AgentWorker } from "./agent.ts";
import { Chat, type Row } from "./chat.ts";
import { COMPAT_PROVIDER, DEFAULT_SETTINGS, checkBaseUrl, loadProviders, masked, secretName, validSettings, type AiSettings } from "./providers.ts";
import { Sessions, group, titleFrom, type Meta } from "./sessions.ts";

const PI_URL = "https://github.com/earendil-works/pi";
/** No event from a running agent for this long: stop and say so. */
export const QUIET_TIMEOUT_MS = 120_000;
/** Streaming text reaches the window at most this often. */
const REFRESH_MS = 100;

const SYSTEM_PROMPT = `你是 Vestra Player AI，内置在 Vestra Player 播放器里的助手，基于 pi 运行。
- 用用户使用的语言回答，默认中文。
- 你可以用 read、write、edit、ls、find、grep 工具读写当前会话的工作文件夹；只能访问这个文件夹，不能运行命令。
- 写 Vestra 插件时遵守插件独立规则：插件只依赖公开的插件 SDK（@vestra-player/sdk，只能 import type），不引用其它插件，不调用系统命令。`;

interface Run {
  worker: AgentWorker;
  meta: Meta;
  chat: Chat;
  running: boolean;
  watchdog?: ReturnType<typeof setTimeout>;
}

/** Vestra Player AI: chat with a model of the user's choice (BYOK); the agent works in the session folder. */
export default async function activate(plugin: Plugin) {
  const providers = loadProviders(import.meta.dirname);
  const sessions = new Sessions(plugin.dataDirectory);
  const settings = plugin.storage.json<AiSettings>("settings", { defaults: DEFAULT_SETTINGS, validate: validSettings });
  const keys = new Map<string, string>(); // provider → key, read from the keychain on demand
  const runs = new Map<string, Run>(); // session id → its worker
  const chats = new Map<string, Chat>(); // session id → rows (loaded once)

  let metas: Meta[] = await sessions.list();
  let active: Meta | null = metas.find((m) => m.id === settings.value.lastSession) ?? null;
  let draft = "";
  let pageMessage = "";
  let pageWarn = false;
  let keyText = "";

  const provider = () => providers.find((p) => p.id === settings.value.provider) ?? providers[0];
  const model = () => settings.value.models[provider().id] || provider().defaultModel;
  const keyFor = async (id: string) => {
    if (!keys.has(id)) keys.set(id, (await plugin.secrets.get(secretName(id))) ?? "");
    return keys.get(id)!;
  };
  await keyFor(provider().id);
  const configured = () => {
    const p = provider();
    if (!model()) return false;
    if (p.id === COMPAT_PROVIDER) return !!settings.value.baseUrl;
    return !!keys.get(p.id);
  };
  const running = () => [...runs.values()].find((r) => r.running);

  /** Throttled window refresh for events that arrive outside SDK handlers. */
  let refresh: ReturnType<typeof setTimeout> | undefined;
  const later = () => {
    refresh ??= setTimeout(() => {
      refresh = undefined;
      plugin.update();
    }, REFRESH_MS);
  };

  const chatOf = async (meta: Meta): Promise<Chat> => {
    let chat = chats.get(meta.id);
    if (!chat) {
      chat = Chat.fromJsonl(await sessions.transcript(meta));
      if (meta.running) {
        // The plugin stopped during a run: the unfinished reply is lost.
        chat.notice("上次的生成被中断（播放器或插件已退出）");
        meta.running = false;
        await sessions.save(meta);
      }
      chats.set(meta.id, chat);
    }
    return chat;
  };
  let shown: Chat = active ? await chatOf(active) : new Chat();

  const sessionRows = () => {
    const rows: { id: string; title: string; header: boolean; active: boolean; running: boolean }[] = [];
    let heading = "";
    for (const meta of metas) {
      const name = group(meta.updated);
      if (name !== heading) rows.push({ id: `h-${name}`, title: (heading = name), header: true, active: false, running: false });
      rows.push({ id: meta.id, title: meta.title, header: false, active: meta.id === active?.id, running: !!runs.get(meta.id)?.running });
    }
    return rows;
  };

  const window = plugin.windows.create("ai.slint");
  window.bind(() => {
    const busy = !!(active && runs.get(active.id)?.running);
    const last = shown.rows.at(-1);
    return {
      sessions: sessionRows(),
      rows: shown.rows.map((row: Row) => ({ ...row })),
      configured: configured(),
      busy,
      blocked: !busy && !!running(),
      sessionTitle: active?.title ?? "新对话",
      modelText: configured() ? `${provider().label} · ${model()}` : "",
      hasFolder: !!active,
      canRetry: !busy && !running() && last?.kind === "error" && !!shown.lastPrompt,
      draft,
    };
  });

  const stopWatchdog = (run: Run) => {
    clearTimeout(run.watchdog);
    run.watchdog = undefined;
  };
  const feed = (run: Run) => {
    stopWatchdog(run);
    if (!run.running) return;
    run.watchdog = setTimeout(() => {
      run.chat.apply({ kind: "settled", aborted: true });
      run.chat.error("timeout", `超过 ${QUIET_TIMEOUT_MS / 1000} 秒没有收到服务商的回复，已停止。`);
      run.worker.abort();
      finish(run);
    }, QUIET_TIMEOUT_MS);
  };
  const finish = (run: Run) => {
    stopWatchdog(run);
    if (!run.running) return;
    run.running = false;
    run.meta.running = false;
    run.meta.updated = Date.now();
    sessions.save(run.meta).catch((error) => console.error(`meta.json: ${error}`));
    later();
  };

  const startWorker = async (meta: Meta, chat: Chat): Promise<Run> => {
    const p = provider();
    const key = await keyFor(p.id);
    const directory = sessions.directory(meta.id);
    const run: Run = {
      meta,
      chat,
      running: false,
      worker: new AgentWorker(
        {
          sessionDir: directory,
          workDir: sessions.work(meta.id),
          sessionFile: meta.file ? path.join(directory, meta.file) : null,
          provider: p.id,
          modelId: model(),
          baseUrl: p.id === COMPAT_PROVIDER ? settings.value.baseUrl : null,
          key,
          systemPrompt: SYSTEM_PROMPT,
        },
        {
          event: (event) => {
            run.chat.apply(event);
            if (event.kind === "settled") finish(run);
            else feed(run);
            later();
          },
          file: (file) => {
            const name = file.split(/[\\/]/).at(-1)!;
            if (run.meta.file !== name) {
              run.meta.file = name;
              sessions.save(run.meta).catch(() => {});
            }
          },
          failed: (message) => {
            if (run.running) {
              run.chat.endTurn();
              run.chat.error(message);
              finish(run);
            }
            later();
          },
          exited: (code) => {
            if (runs.get(meta.id) === run) runs.delete(meta.id);
            if (run.running) {
              run.chat.endTurn();
              run.chat.error(run.worker.lastError() || `exit ${code}`, "agent 进程意外退出。");
              finish(run);
            }
            later();
          },
        },
      ),
    };
    runs.set(meta.id, run);
    return run;
  };

  /** Ends the workers that are not generating (another session shown, settings changed). */
  const releaseIdle = (except?: string) => {
    for (const [id, run] of runs)
      if (!run.running && id !== except) {
        runs.delete(id);
        run.worker.stop().catch(() => {});
      }
  };

  const send = async (text: string) => {
    const message = text.trim();
    if (!message) return;
    if (!configured()) throw Error("请先在设置中配置服务商和 API key");
    if (running()) throw Error("另一会话正在生成，请稍候");
    if (!active) {
      active = await sessions.create(titleFrom(message));
      chats.set(active.id, shown);
      settings.save({ ...settings.value, lastSession: active.id });
    }
    const meta = active;
    const chat = shown;
    draft = "";
    window.set({ draft: "" }); // clears the field even if the user typed the same text again
    chat.user(message);
    let run = runs.get(meta.id);
    if (!run || run.worker.exited) run = await startWorker(meta, chat);
    run.running = true;
    meta.running = true;
    meta.updated = Date.now();
    await sessions.save(meta);
    metas = [meta, ...metas.filter((m) => m.id !== meta.id)];
    feed(run);
    run.worker.prompt(message).catch(() => {}); // start failures arrive through `failed`
  };

  const select = async (meta: Meta | null) => {
    active = meta;
    shown = meta ? await chatOf(meta) : new Chat();
    releaseIdle(meta?.id);
    settings.save({ ...settings.value, lastSession: meta?.id ?? null });
  };

  window.on("send", send);
  window.on("draftChanged", (text: string) => {
    draft = text;
  });
  window.on("stop", () => {
    const run = active && runs.get(active.id);
    if (run?.running) run.worker.abort();
  });
  window.on("retry", () => send(shown.lastPrompt));
  window.on("newChat", () => select(null));
  window.on("selectSession", (id: string) => select(metas.find((m) => m.id === id) ?? null));
  window.on("toggle", (id: string) => shown.toggle(id));
  window.on("copy", (id: string) => {
    const row = shown.rows.find((r) => r.id === id);
    if (row) return plugin.clipboard.writeText(row.text);
  });
  window.on("openSettings", () => plugin.settings.show("byok"));
  window.on("openFolder", () => active && plugin.system.openPath(sessions.work(active.id)));
  window.on("openPi", () => plugin.system.openUrl(PI_URL));

  // ---- settings page (Options ▸ 插件 ▸ Vestra Player AI)
  const page = plugin.settings.page("settings.slint", { id: "byok", category: "plugins", title: "Vestra Player AI" });
  page.bind(() => {
    const p = provider();
    const key = keys.get(p.id) ?? "";
    return {
      providers: providers.map((x) => x.label),
      providerIndex: providers.indexOf(p),
      compat: p.id === COMPAT_PROVIDER,
      baseUrl: settings.value.baseUrl,
      keyText,
      keyState: p.keyOptional && !key ? "未保存（本地服务可以不填）" : masked(key),
      hasKey: !!key,
      models: p.models,
      modelIndex: Math.max(0, p.models.indexOf(model())),
      modelText: model(),
      message: pageMessage,
      messageWarn: pageWarn,
    };
  });
  const say = (text: string, warn = false) => {
    pageMessage = text;
    pageWarn = warn;
  };
  /** A change of service, key or model applies to the next message. */
  const changed = () => releaseIdle();
  page.on("chooseProvider", async (index: number) => {
    const p = providers[index];
    if (!p) return;
    settings.save({ ...settings.value, provider: p.id });
    keyText = "";
    await keyFor(p.id);
    say("");
    changed();
  });
  page.on("saveKey", async (text: string) => {
    const key = text.trim();
    if (!key) return;
    const p = provider();
    try {
      await plugin.secrets.set(secretName(p.id), key);
    } catch (error) {
      say(`没能保存到钥匙串：${(error as Error).message}`, true);
      return;
    }
    keys.set(p.id, key);
    keyText = "";
    page.set({ keyText: "" });
    say(`${p.label} 的 key 已保存。`);
    changed();
  });
  page.on("deleteKey", async () => {
    const p = provider();
    await plugin.secrets.delete(secretName(p.id));
    keys.set(p.id, "");
    say(`${p.label} 的 key 已删除。`);
    changed();
  });
  page.on("setBaseUrl", (text: string) => {
    try {
      settings.save({ ...settings.value, baseUrl: text.trim() ? checkBaseUrl(text) : "" });
      say("");
    } catch (error) {
      say((error as Error).message, true);
    }
    changed();
  });
  page.on("chooseModel", (id: string) => {
    settings.save({ ...settings.value, models: { ...settings.value.models, [provider().id]: id.trim() } });
    changed();
  });

  plugin.onDispose(async () => {
    clearTimeout(refresh);
    // Runs still in progress stay marked in meta.json: the next start says so.
    await Promise.all([...runs.values()].map((run) => (stopWatchdog(run), run.worker.stop(300))));
  });

  plugin.commands.add({
    id: "workshop.open",
    path: ["Vestra Player AI…"],
    order: 820,
    group: "windows",
    run: () => window.show(),
  });
}
